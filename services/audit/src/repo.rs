//! Postgres access for audit-service: the append transaction, one page of a
//! subject's decisions, and the ascending walk the verifier needs.
//! Runtime-checked queries (`query_as` + `FromRow`) with literal SQL, as in
//! policy-service — sqlx 0.9 only accepts `&'static str` SQL, so the column
//! list is spelled out per query.

use afixo_common::time::to_proto;
use afixo_proto::DecisionRecord;
use sqlx::{FromRow, PgConnection, PgPool};
use time::OffsetDateTime;
use uuid::Uuid;

use crate::chain::{self, Committed};

/// Key of the transaction-scoped advisory lock every append takes first (the
/// bytes `afixoaud`). It is what actually serialises appenders — see [`append`].
const CHAIN_LOCK: i64 = i64::from_be_bytes(*b"afixoaud");

/// A decision as the chain stores it: ids parsed, the optional ones `None`
/// where the proto carried an empty string. The server builds it from a
/// validated `DisclosureDecided`; every row reads back into one (flattened
/// into [`DecisionRow`]), so append and verify hash exactly the same thing.
#[derive(Debug, Clone, PartialEq, Eq, FromRow)]
pub struct Decision {
    pub event_id: Uuid,
    pub subject_id: Uuid,
    pub requester_id: Uuid,
    pub purpose: String,
    pub allowed: bool,
    pub reason: String,
    pub persona_id: Option<Uuid>,
    pub persona_label: Option<String>,
    pub rule_id: Option<Uuid>,
    pub disclosed_keys: Vec<String>,
    pub withheld_keys: Vec<String>,
    pub decided_at: OffsetDateTime,
}

impl Decision {
    /// The chain hash of this decision appended after `prev`. Ids are hashed
    /// in their hyphenated lower-case form and absent values as empty strings
    /// — the form a row reads back in — so the verifier can recompute the
    /// hash from the table alone.
    pub fn row_hash(&self, prev: &[u8; 32]) -> [u8; 32] {
        let event_id = self.event_id.to_string();
        let subject_id = self.subject_id.to_string();
        let requester_id = self.requester_id.to_string();
        let persona_id = opt_id(self.persona_id);
        let rule_id = opt_id(self.rule_id);
        let decided_at = to_proto(self.stored_decided_at());
        chain::row_hash(
            prev,
            &Committed {
                event_id: &event_id,
                subject_id: &subject_id,
                requester_id: &requester_id,
                purpose: &self.purpose,
                allowed: self.allowed,
                reason: &self.reason,
                persona_id: &persona_id,
                persona_label: self.persona_label.as_deref().unwrap_or_default(),
                rule_id: &rule_id,
                disclosed_keys: &self.disclosed_keys,
                withheld_keys: &self.withheld_keys,
                decided_at: (decided_at.seconds, decided_at.nanos),
            },
        )
    }

    /// `decided_at` as Postgres will hold it. `timestamptz` keeps
    /// microseconds, and the hash must commit to the stored value: a decision
    /// stamped with nanoseconds would otherwise never verify again.
    pub fn stored_decided_at(&self) -> OffsetDateTime {
        let nanos = self.decided_at.nanosecond();
        self.decided_at
            .replace_nanosecond(nanos - nanos % 1_000)
            .expect("fewer nanoseconds than before stay in range")
    }
}

fn opt_id(id: Option<Uuid>) -> String {
    id.map_or_else(String::new, |u| u.to_string())
}

/// One row of `disclosure_events`.
#[derive(Debug, Clone, FromRow)]
pub struct DecisionRow {
    pub seq: i64,
    #[sqlx(flatten)]
    pub decision: Decision,
    pub recorded_at: OffsetDateTime,
    pub prev_hash: [u8; 32],
    pub hash: [u8; 32],
}

impl From<DecisionRow> for DecisionRecord {
    fn from(r: DecisionRow) -> Self {
        let d = r.decision;
        Self {
            seq: r.seq,
            event_id: d.event_id.to_string(),
            subject_id: d.subject_id.to_string(),
            requester_id: d.requester_id.to_string(),
            purpose: d.purpose,
            allowed: d.allowed,
            reason: d.reason,
            persona_id: opt_id(d.persona_id),
            persona_label: d.persona_label.unwrap_or_default(),
            disclosed_keys: d.disclosed_keys,
            withheld_keys: d.withheld_keys,
            decided_at: Some(to_proto(d.decided_at)),
            recorded_at: Some(to_proto(r.recorded_at)),
            hash: r.hash.to_vec(),
            prev_hash: r.prev_hash.to_vec(),
        }
    }
}

/// Append one decision and return its `(seq, hash)` — or, when a row with the
/// same `event_id` already exists (a retry), that row's, unchanged.
///
/// One transaction. Appenders are serialised by a transaction-scoped advisory
/// lock taken before the head is read. The `for update` lock on the head row
/// stays as the second line of defence, but on its own it is not enough: under
/// READ COMMITTED a second appender that blocked on it resumes with the
/// snapshot its statement started with, never sees the row the first one
/// inserted, and would chain onto the same predecessor.
pub async fn append(pool: &PgPool, d: &Decision) -> sqlx::Result<(i64, [u8; 32])> {
    let mut tx = pool.begin().await?;
    sqlx::query("select pg_advisory_xact_lock($1)")
        .bind(CHAIN_LOCK)
        .execute(&mut *tx)
        .await?;
    let prev: Option<[u8; 32]> = sqlx::query_scalar(
        "select hash from disclosure_events order by seq desc limit 1 for update",
    )
    .fetch_optional(&mut *tx)
    .await?;
    let prev = prev.unwrap_or(chain::GENESIS);
    let hash = d.row_hash(&prev);
    let inserted: Option<(i64, [u8; 32])> = sqlx::query_as(
        "insert into disclosure_events \
            (event_id, subject_id, requester_id, purpose, allowed, reason, persona_id, \
             persona_label, rule_id, disclosed_keys, withheld_keys, decided_at, prev_hash, hash) \
         values ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10, $11, $12, $13, $14) \
         on conflict (event_id) do nothing \
         returning seq, hash",
    )
    .bind(d.event_id)
    .bind(d.subject_id)
    .bind(d.requester_id)
    .bind(d.purpose.as_str())
    .bind(d.allowed)
    .bind(d.reason.as_str())
    .bind(d.persona_id)
    .bind(d.persona_label.as_deref())
    .bind(d.rule_id)
    .bind(d.disclosed_keys.as_slice())
    .bind(d.withheld_keys.as_slice())
    .bind(d.stored_decided_at())
    .bind(prev)
    .bind(hash)
    .fetch_optional(&mut *tx)
    .await?;
    let row = match inserted {
        Some(fresh) => fresh,
        None => existing(&mut tx, d.event_id).await?,
    };
    tx.commit().await?;
    Ok(row)
}

/// The row a duplicate `event_id` collided with.
async fn existing(conn: &mut PgConnection, event_id: Uuid) -> sqlx::Result<(i64, [u8; 32])> {
    let row: (i64, [u8; 32]) =
        sqlx::query_as("select seq, hash from disclosure_events where event_id = $1")
            .bind(event_id)
            .fetch_one(&mut *conn)
            .await?;
    tracing::info!(%event_id, seq = row.0, "event already recorded; returning the existing row");
    Ok(row)
}

/// One page of a subject's decisions, newest first. `before_seq` is the keyset
/// cursor: only rows with `seq < before_seq`.
pub async fn list(
    pool: &PgPool,
    subject_id: Uuid,
    limit: u16,
    before_seq: Option<i64>,
) -> sqlx::Result<Vec<DecisionRow>> {
    sqlx::query_as(
        "select seq, event_id, subject_id, requester_id, purpose, allowed, reason, persona_id, \
                persona_label, rule_id, disclosed_keys, withheld_keys, decided_at, recorded_at, \
                prev_hash, hash \
           from disclosure_events \
          where subject_id = $1 and seq < $2 \
          order by seq desc \
          limit $3",
    )
    .bind(subject_id)
    .bind(before_seq.unwrap_or(i64::MAX))
    .bind(i64::from(limit))
    .fetch_all(pool)
    .await
}

/// Every row in chain order, for the verifier. Loads the whole table — O(n)
/// in memory as well as time, which is fine at this scale (CLAUDE.md); switch
/// to a `fetch` stream before the table outgrows that.
pub async fn all_ascending(pool: &PgPool) -> sqlx::Result<Vec<DecisionRow>> {
    sqlx::query_as(
        "select seq, event_id, subject_id, requester_id, purpose, allowed, reason, persona_id, \
                persona_label, rule_id, disclosed_keys, withheld_keys, decided_at, recorded_at, \
                prev_hash, hash \
           from disclosure_events \
          order by seq asc",
    )
    .fetch_all(pool)
    .await
}
