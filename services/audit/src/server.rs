//! gRPC surface of audit-service: `Record` (validate, append), `ListDecisions`
//! (one keyset page of a subject's decisions) and `VerifyChain` (recompute
//! every hash in `seq` order).

use afixo_common::{grpc, ids, time::from_proto};
use afixo_proto::{
    DisclosureDecided, ListDecisionsRequest, ListDecisionsResponse, RecordDecisionRequest,
    RecordDecisionResponse, VerifyChainRequest, VerifyChainResponse,
    audit_service_server::AuditService,
};
use sqlx::PgPool;
use tonic::{Request, Response, Status};

use crate::{
    chain,
    repo::{self, Decision, DecisionRow},
};

/// `ListDecisions.limit` when the request leaves it at 0.
const DEFAULT_PAGE: u16 = 50;
/// The most rows one `ListDecisions` page may carry.
const MAX_PAGE: u16 = 200;

#[derive(Debug, Clone)]
pub struct AuditSvc {
    pool: PgPool,
}

impl AuditSvc {
    pub fn new(pool: PgPool) -> Self {
        Self { pool }
    }
}

/// Validate a `DisclosureDecided` into what the chain stores. Every id must be
/// a uuid — the zero uuid is a legitimate `subject_id` (an unknown-handle
/// probe) — `persona_id` and `rule_id` may be empty; `decided_at` is required.
fn parse_decision(d: DisclosureDecided) -> Result<Decision, Status> {
    let decided_at = match &d.decided_at {
        None => return Err(Status::invalid_argument("decided_at: required")),
        Some(t) => from_proto(t)
            .ok_or_else(|| Status::invalid_argument("decided_at: not a valid timestamp"))?,
    };
    Ok(Decision {
        event_id: ids::parse("event_id", &d.event_id)?,
        subject_id: ids::parse("subject_id", &d.subject_id)?,
        requester_id: ids::parse("requester_id", &d.requester_id)?,
        persona_id: ids::parse_opt("persona_id", Some(d.persona_id.as_str()))?,
        rule_id: ids::parse_opt("rule_id", Some(d.rule_id.as_str()))?,
        persona_label: Some(d.persona_label).filter(|label| !label.is_empty()),
        purpose: d.purpose,
        allowed: d.allowed,
        reason: d.reason,
        disclosed_keys: d.disclosed_keys,
        withheld_keys: d.withheld_keys,
        decided_at,
    })
}

/// `limit` as the proto defines it: 0 (unset) is the default, anything else
/// is clamped to `1..=MAX_PAGE`.
fn page_limit(raw: i32) -> u16 {
    if raw == 0 {
        DEFAULT_PAGE
    } else {
        u16::try_from(raw.clamp(1, i32::from(MAX_PAGE))).unwrap_or(MAX_PAGE)
    }
}

/// Walk the rows in `seq` order, recomputing every hash from the row itself
/// and its predecessor's; the first row whose `prev_hash` or `hash` disagrees
/// is the break. `head_hash` is the stored head — genesis for an empty table.
fn verify(rows: &[DecisionRow]) -> VerifyChainResponse {
    let mut prev = chain::GENESIS;
    let mut broken_at_seq = None;
    for row in rows {
        if row.prev_hash != prev || row.hash != row.decision.row_hash(&prev) {
            broken_at_seq = Some(row.seq);
            break;
        }
        prev = row.hash;
    }
    VerifyChainResponse {
        ok: broken_at_seq.is_none(),
        length: i64::try_from(rows.len()).unwrap_or(i64::MAX),
        broken_at_seq,
        head_hash: rows.last().map_or(chain::GENESIS, |r| r.hash).to_vec(),
    }
}

#[tonic::async_trait]
impl AuditService for AuditSvc {
    async fn record(
        &self,
        req: Request<RecordDecisionRequest>,
    ) -> Result<Response<RecordDecisionResponse>, Status> {
        let decision = req
            .into_inner()
            .decision
            .ok_or_else(|| Status::invalid_argument("decision: required"))?;
        let decision = parse_decision(decision)?;
        let (seq, hash) = repo::append(&self.pool, &decision)
            .await
            .map_err(|e| grpc::db_status("record decision", e))?;
        tracing::info!(
            seq,
            event_id = %decision.event_id,
            allowed = decision.allowed,
            "decision recorded"
        );
        Ok(Response::new(RecordDecisionResponse {
            seq,
            hash: hash.to_vec(),
        }))
    }

    async fn list_decisions(
        &self,
        req: Request<ListDecisionsRequest>,
    ) -> Result<Response<ListDecisionsResponse>, Status> {
        let req = req.get_ref();
        let subject_id = ids::parse("subject_id", &req.subject_id)?;
        let limit = page_limit(req.limit);
        let rows = repo::list(&self.pool, subject_id, limit, req.before_seq)
            .await
            .map_err(|e| grpc::db_status("list decisions", e))?;
        // A full page may also be the last one; the caller then gets one empty page.
        let next_before_seq = if rows.len() == usize::from(limit) {
            rows.last().map(|r| r.seq)
        } else {
            None
        };
        let decisions = rows.into_iter().map(Into::into).collect();
        Ok(Response::new(ListDecisionsResponse {
            decisions,
            next_before_seq,
        }))
    }

    async fn verify_chain(
        &self,
        _req: Request<VerifyChainRequest>,
    ) -> Result<Response<VerifyChainResponse>, Status> {
        let rows = repo::all_ascending(&self.pool)
            .await
            .map_err(|e| grpc::db_status("verify chain", e))?;
        let report = verify(&rows);
        if report.ok {
            tracing::info!(
                length = report.length,
                head = %hex::encode(&report.head_hash),
                "chain verified"
            );
        } else {
            tracing::error!(
                length = report.length,
                broken_at_seq = ?report.broken_at_seq,
                "chain broken"
            );
        }
        Ok(Response::new(report))
    }
}

#[cfg(test)]
mod tests {
    use afixo_common::ids::new_id;
    use time::OffsetDateTime;

    use super::*;

    fn decision() -> Decision {
        Decision {
            event_id: new_id(),
            subject_id: new_id(),
            requester_id: new_id(),
            purpose: "shipping".to_owned(),
            allowed: true,
            reason: "rule_matched".to_owned(),
            persona_id: Some(new_id()),
            persona_label: Some("legal".to_owned()),
            rule_id: Some(new_id()),
            disclosed_keys: vec!["full_name".to_owned()],
            withheld_keys: vec!["phone".to_owned()],
            decided_at: OffsetDateTime::UNIX_EPOCH,
        }
    }

    /// `n` rows chained from genesis, as the table would hold them.
    fn chain_of(n: i64) -> Vec<DecisionRow> {
        let mut prev = chain::GENESIS;
        (1..=n)
            .map(|seq| {
                let decision = decision();
                let hash = decision.row_hash(&prev);
                let row = DecisionRow {
                    seq,
                    decision,
                    recorded_at: OffsetDateTime::UNIX_EPOCH,
                    prev_hash: prev,
                    hash,
                };
                prev = hash;
                row
            })
            .collect()
    }

    #[test]
    fn limit_defaults_and_clamps() {
        assert_eq!(page_limit(0), DEFAULT_PAGE);
        assert_eq!(page_limit(7), 7);
        assert_eq!(page_limit(-3), 1);
        assert_eq!(page_limit(1_000), MAX_PAGE);
    }

    #[test]
    fn empty_chain_verifies_with_the_genesis_head() {
        let report = verify(&[]);
        assert!(report.ok);
        assert_eq!(report.length, 0);
        assert_eq!(report.broken_at_seq, None);
        assert_eq!(report.head_hash, chain::GENESIS);
    }

    #[test]
    fn intact_chain_verifies() {
        let rows = chain_of(3);
        let report = verify(&rows);
        assert!(report.ok);
        assert_eq!(report.length, 3);
        assert_eq!(report.head_hash, rows[2].hash);
    }

    #[test]
    fn edited_row_breaks_at_that_row() {
        let mut rows = chain_of(3);
        rows[1].decision.allowed = false;
        let report = verify(&rows);
        assert!(!report.ok);
        assert_eq!(report.broken_at_seq, Some(2));
        assert_eq!(report.length, 3);
        assert_eq!(report.head_hash, rows[2].hash);
    }

    #[test]
    fn removed_row_breaks_at_the_next_one() {
        let mut rows = chain_of(3);
        rows.remove(1);
        let report = verify(&rows);
        assert!(!report.ok);
        assert_eq!(report.broken_at_seq, Some(3));
        assert_eq!(report.length, 2);
    }

    #[test]
    fn forged_hash_breaks_at_that_row() {
        let mut rows = chain_of(2);
        rows[1].hash = [7; 32];
        assert_eq!(verify(&rows).broken_at_seq, Some(2));
    }
}
