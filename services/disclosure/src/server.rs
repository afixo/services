//! gRPC surface of disclosure-service: the orchestration fixed in CLAUDE.md.
//!
//! ```text
//! resolve subject → candidate rules → select → persona → decide → audit.Record (awaited) → answer
//! ```

use std::{collections::HashMap, time::Duration};

use afixo_common::{grpc, ids, time::to_proto};
use afixo_engine::{CandidateRule, Decision, DenyReason, Persona, decide, select_rule};
use afixo_proto::{
    DiscloseRequest, DiscloseResponse, DisclosureDecided, GetPersonaRequest,
    ListCandidateRulesRequest, RecordDecisionRequest, ResolveSubjectRequest,
    audit_service_client::AuditServiceClient, disclosure_service_server::DisclosureService,
    identity_service_client::IdentityServiceClient, policy_service_client::PolicyServiceClient,
};
use time::OffsetDateTime;
use tonic::{Code, Request, Response, Status, transport::Channel};
use uuid::Uuid;

/// Per upstream call.
const UPSTREAM_TIMEOUT: Duration = Duration::from_secs(2);

#[derive(Debug, Clone)]
pub struct DisclosureSvc {
    identity: IdentityServiceClient<Channel>,
    policy: PolicyServiceClient<Channel>,
    audit: AuditServiceClient<Channel>,
}

impl DisclosureSvc {
    pub fn new(identity: Channel, policy: Channel, audit: Channel) -> Self {
        Self {
            identity: IdentityServiceClient::new(identity),
            policy: PolicyServiceClient::new(policy),
            audit: AuditServiceClient::new(audit),
        }
    }

    /// Steps 1–5: everything up to (and including) the engine's verdict.
    async fn decide(
        &self,
        handle: &str,
        requester_id: Uuid,
        purpose: &str,
    ) -> Result<(Uuid, Decision), Status> {
        let subject = match with_timeout(
            "identity.ResolveSubject",
            self.identity
                .clone()
                .resolve_subject(timed(ResolveSubjectRequest {
                    handle: handle.to_owned(),
                })),
        )
        .await
        {
            Ok(s) => s,
            Err(s) if s.code() == Code::NotFound || s.code() == Code::InvalidArgument => {
                return Ok((
                    Uuid::nil(),
                    Decision::Denied {
                        reason: DenyReason::NoMatchingRule,
                        rule_id: None,
                    },
                ));
            }
            Err(s) => return Err(upstream("identity.ResolveSubject", s)),
        };
        let subject_id = ids::parse("subject.id", &subject.id)?;

        // An unknown purpose is the caller's error (400), not a deny.
        let candidates: Vec<CandidateRule> = with_timeout(
            "policy.ListCandidateRules",
            self.policy
                .clone()
                .list_candidate_rules(timed(ListCandidateRulesRequest {
                    subject_id: subject.id.clone(),
                    requester_id: requester_id.to_string(),
                    purpose: purpose.to_owned(),
                })),
        )
        .await
        .map_err(|s| match s.code() {
            Code::InvalidArgument => s,
            _ => upstream("policy.ListCandidateRules", s),
        })?
        .rules
        .iter()
        .map(crate::convert::candidate)
        .collect::<Result<_, _>>()?;

        let winner = select_rule(candidates);
        let persona: Option<Persona> = match &winner {
            None => None,
            Some(rule) => match with_timeout(
                "identity.GetPersona",
                self.identity.clone().get_persona(timed(GetPersonaRequest {
                    persona_id: rule.persona_id.to_string(),
                    subject_id: None,
                })),
            )
            .await
            {
                Ok(p) => Some(crate::convert::persona(p)?),
                Err(s) if s.code() == Code::NotFound => None,
                Err(s) => return Err(upstream("identity.GetPersona", s)),
            },
        };

        Ok((subject_id, decide(subject_id, winner, persona)))
    }

    /// Step 6: record the decision, retrying once on a transient failure.
    /// Never answer the caller unless this succeeded.
    async fn record(&self, event: DisclosureDecided) -> Result<(), Status> {
        let mut last = None;
        for attempt in 0..2 {
            let mut audit = self.audit.clone();
            let call = audit.record(timed(RecordDecisionRequest {
                decision: Some(event.clone()),
            }));
            match tokio::time::timeout(UPSTREAM_TIMEOUT, call).await {
                Ok(Ok(_)) => return Ok(()),
                Ok(Err(s)) if matches!(s.code(), Code::Unavailable | Code::DeadlineExceeded) => {
                    tracing::warn!(attempt, code = ?s.code(), "audit.Record failed; retrying");
                    last = Some(s);
                }
                Ok(Err(s)) => return Err(grpc::internal("audit.Record", s)),
                Err(_) => {
                    tracing::warn!(attempt, "audit.Record timed out; retrying");
                    last = Some(Status::deadline_exceeded("audit.Record"));
                }
            }
        }
        tracing::error!(event_id = %event.event_id, "audit unavailable; refusing to answer unrecorded");
        Err(Status::unavailable(format!(
            "audit unavailable: {}",
            last.map(|s| s.message().to_owned()).unwrap_or_default()
        )))
    }
}

/// What a decision looks like on the wire — for the audit event (keys only)
/// and for the response (keys and values).
struct Outcome {
    allowed: bool,
    reason: String,
    persona_id: String,
    persona_label: String,
    rule_id: String,
    disclosed_keys: Vec<String>,
    withheld_keys: Vec<String>,
    fields: HashMap<String, String>,
}

fn outcome(decision: Decision, subject_known: bool) -> Outcome {
    match decision {
        Decision::Allowed {
            rule_id,
            persona_id,
            persona_label,
            disclosed,
            withheld,
        } => Outcome {
            allowed: true,
            reason: "rule_matched".to_owned(),
            persona_id: persona_id.to_string(),
            persona_label,
            rule_id: rule_id.to_string(),
            disclosed_keys: disclosed.iter().map(|f| f.key.clone()).collect(),
            withheld_keys: withheld,
            fields: disclosed.into_iter().map(|f| (f.key, f.value)).collect(),
        },
        Decision::Denied { reason, rule_id } => Outcome {
            allowed: false,
            reason: if subject_known {
                reason.as_str().to_owned()
            } else {
                "unknown_subject".to_owned()
            },
            persona_id: String::new(),
            persona_label: String::new(),
            rule_id: rule_id.map(|r| r.to_string()).unwrap_or_default(),
            disclosed_keys: vec![],
            withheld_keys: vec![],
            fields: HashMap::new(),
        },
    }
}

fn timed<T>(msg: T) -> Request<T> {
    let mut req = Request::new(msg);
    req.set_timeout(UPSTREAM_TIMEOUT);
    req
}

async fn with_timeout<T>(
    what: &'static str,
    fut: impl Future<Output = Result<Response<T>, Status>>,
) -> Result<T, Status> {
    match tokio::time::timeout(UPSTREAM_TIMEOUT, fut).await {
        Ok(Ok(resp)) => Ok(resp.into_inner()),
        Ok(Err(s)) => Err(s),
        Err(_) => Err(Status::deadline_exceeded(what)),
    }
}

/// Upstream errors that are not semantic become UNAVAILABLE/INTERNAL, never a deny.
fn upstream(what: &'static str, s: Status) -> Status {
    match s.code() {
        Code::Unavailable | Code::DeadlineExceeded => {
            tracing::warn!(what, code = ?s.code(), "upstream unavailable");
            Status::unavailable(what)
        }
        _ => grpc::internal(what, s),
    }
}

#[tonic::async_trait]
impl DisclosureService for DisclosureSvc {
    async fn disclose(
        &self,
        req: Request<DiscloseRequest>,
    ) -> Result<Response<DiscloseResponse>, Status> {
        let req = req.into_inner();
        let requester_id = ids::parse("requester_id", &req.requester_id)?;
        if req.purpose.is_empty() {
            return Err(Status::invalid_argument("purpose: required"));
        }
        let handle = req.subject_handle.trim().to_lowercase();
        let decided_at = OffsetDateTime::now_utc();
        let event_id = ids::new_id();

        let (subject_id, decision) = self.decide(&handle, requester_id, &req.purpose).await?;
        let out = outcome(decision, !subject_id.is_nil());

        // Awaited: the response below is unreachable without a recorded decision.
        self.record(DisclosureDecided {
            event_id: event_id.to_string(),
            subject_id: subject_id.to_string(),
            requester_id: requester_id.to_string(),
            purpose: req.purpose.clone(),
            allowed: out.allowed,
            reason: out.reason.clone(),
            persona_id: out.persona_id,
            persona_label: out.persona_label.clone(),
            rule_id: out.rule_id,
            disclosed_keys: out.disclosed_keys,
            withheld_keys: out.withheld_keys.clone(),
            decided_at: Some(to_proto(decided_at)),
        })
        .await?;

        tracing::info!(
            %event_id, %requester_id, purpose = %req.purpose, allowed = out.allowed, reason = %out.reason,
            "decision recorded"
        );

        Ok(Response::new(DiscloseResponse {
            decision_id: event_id.to_string(),
            allowed: out.allowed,
            reason: out.reason,
            persona_label: out.persona_label,
            fields: out.fields,
            withheld_keys: out.withheld_keys,
            decided_at: Some(to_proto(decided_at)),
        }))
    }
}
