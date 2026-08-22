//! Integration tests for audit-service against Postgres. `#[sqlx::test]`
//! gives every test its own database, created from `DATABASE_URL` with the
//! migrations applied; the service struct is driven through the
//! `AuditService` trait exactly as tonic would.
//!
//! ```sh
//! docker compose up -d postgres
//! DATABASE_URL=postgres://afixo:afixo@localhost:5432/afixo cargo test -p afixo-audit
//! ```

use afixo_audit::{chain, server::AuditSvc};
use afixo_common::{
    ids,
    time::{now, to_proto},
};
use afixo_proto::{
    DisclosureDecided, ListDecisionsRequest, ListDecisionsResponse, RecordDecisionRequest,
    RecordDecisionResponse, VerifyChainRequest, VerifyChainResponse,
    audit_service_server::AuditService,
};
use sqlx::PgPool;
use tonic::{Code, Request, Response, Status};
use uuid::Uuid;

/// A plausible allowed decision for `subject_id`, stamped at microsecond
/// precision (what Postgres stores, so the hash can be recomputed by hand).
fn decision(subject_id: Uuid) -> DisclosureDecided {
    let mut decided_at = to_proto(now());
    decided_at.nanos = 123_456_000;
    DisclosureDecided {
        event_id: ids::new_id().to_string(),
        subject_id: subject_id.to_string(),
        requester_id: ids::new_id().to_string(),
        purpose: "shipping".to_owned(),
        allowed: true,
        reason: "rule_matched".to_owned(),
        persona_id: ids::new_id().to_string(),
        persona_label: "legal".to_owned(),
        rule_id: ids::new_id().to_string(),
        disclosed_keys: vec!["full_name".to_owned(), "street".to_owned()],
        withheld_keys: vec!["phone".to_owned()],
        decided_at: Some(decided_at),
    }
}

/// The canonical view of a request, as `docs/audit.md` specifies it.
fn committed(d: &DisclosureDecided) -> chain::Committed<'_> {
    let ts = d.decided_at.as_ref().expect("decided_at set");
    chain::Committed {
        event_id: &d.event_id,
        subject_id: &d.subject_id,
        requester_id: &d.requester_id,
        purpose: &d.purpose,
        allowed: d.allowed,
        reason: &d.reason,
        persona_id: &d.persona_id,
        persona_label: &d.persona_label,
        rule_id: &d.rule_id,
        disclosed_keys: &d.disclosed_keys,
        withheld_keys: &d.withheld_keys,
        decided_at: (ts.seconds, ts.nanos),
    }
}

async fn record(svc: &AuditSvc, d: DisclosureDecided) -> Result<RecordDecisionResponse, Status> {
    svc.record(Request::new(RecordDecisionRequest { decision: Some(d) }))
        .await
        .map(Response::into_inner)
}

async fn list(
    svc: &AuditSvc,
    subject_id: Uuid,
    limit: i32,
    before_seq: Option<i64>,
) -> Result<ListDecisionsResponse, Status> {
    svc.list_decisions(Request::new(ListDecisionsRequest {
        subject_id: subject_id.to_string(),
        limit,
        before_seq,
    }))
    .await
    .map(Response::into_inner)
}

async fn verify(svc: &AuditSvc) -> VerifyChainResponse {
    svc.verify_chain(Request::new(VerifyChainRequest {}))
        .await
        .expect("VerifyChain answers")
        .into_inner()
}

fn seqs(page: &ListDecisionsResponse) -> Vec<i64> {
    page.decisions.iter().map(|d| d.seq).collect()
}

async fn row_count(pool: &PgPool) -> i64 {
    sqlx::query_scalar("select count(*) from disclosure_events")
        .fetch_one(pool)
        .await
        .expect("count rows")
}

/// What a database superuser can do to the table: the trigger is only
/// defence in depth against our own bugs.
async fn without_trigger(pool: &PgPool, sql: &'static str) -> sqlx::Result<()> {
    sqlx::query("alter table disclosure_events disable trigger disclosure_events_no_update")
        .execute(pool)
        .await?;
    sqlx::query(sql).execute(pool).await?;
    sqlx::query("alter table disclosure_events enable trigger disclosure_events_no_update")
        .execute(pool)
        .await?;
    Ok(())
}

#[sqlx::test(migrations = "./migrations")]
async fn first_record_starts_the_chain(pool: PgPool) -> anyhow::Result<()> {
    let svc = AuditSvc::new(pool);
    let subject = ids::new_id();
    let d = decision(subject);

    let res = record(&svc, d.clone()).await?;
    assert_eq!(res.seq, 1);
    assert_eq!(res.hash.len(), 32);
    assert_ne!(res.hash, chain::GENESIS);
    // The stored hash is the documented canonical encoding of the request itself.
    assert_eq!(res.hash, chain::row_hash(&chain::GENESIS, &committed(&d)));

    let page = list(&svc, subject, 0, None).await?;
    assert_eq!(page.decisions.len(), 1);
    assert_eq!(page.next_before_seq, None);
    let row = &page.decisions[0];
    assert_eq!(row.seq, 1);
    assert_eq!(row.prev_hash, chain::GENESIS);
    assert_eq!(row.hash, res.hash);
    assert_eq!(row.event_id, d.event_id);
    assert_eq!(row.subject_id, d.subject_id);
    assert_eq!(row.requester_id, d.requester_id);
    assert_eq!(row.persona_id, d.persona_id);
    assert_eq!(row.persona_label, d.persona_label);
    assert_eq!(row.disclosed_keys, d.disclosed_keys);
    assert_eq!(row.withheld_keys, d.withheld_keys);
    assert_eq!(row.decided_at, d.decided_at);
    assert!(row.recorded_at.is_some());

    let report = verify(&svc).await;
    assert!(report.ok);
    assert_eq!(report.length, 1);
    assert_eq!(report.broken_at_seq, None);
    assert_eq!(report.head_hash, res.hash);
    Ok(())
}

#[sqlx::test(migrations = "./migrations")]
async fn record_is_idempotent_on_event_id(pool: PgPool) -> anyhow::Result<()> {
    let svc = AuditSvc::new(pool.clone());
    let d = decision(ids::new_id());

    let first = record(&svc, d.clone()).await?;
    let retry = record(&svc, d.clone()).await?;
    assert_eq!(retry.seq, first.seq);
    assert_eq!(retry.hash, first.hash);

    // A replay is not re-examined: the first write is the record.
    let mut altered = d;
    altered.allowed = false;
    let again = record(&svc, altered).await?;
    assert_eq!(again.seq, first.seq);
    assert_eq!(again.hash, first.hash);
    assert_eq!(row_count(&pool).await, 1);

    // And the chain continues from the one row that exists.
    let next = record(&svc, decision(ids::new_id())).await?;
    assert!(next.seq > first.seq);
    let report = verify(&svc).await;
    assert!(report.ok);
    assert_eq!(report.length, 2);
    Ok(())
}

#[sqlx::test(migrations = "./migrations")]
async fn rows_chain_onto_each_other(pool: PgPool) -> anyhow::Result<()> {
    let svc = AuditSvc::new(pool);
    let subject = ids::new_id();
    let first = record(&svc, decision(subject)).await?;

    let mut denied = decision(subject);
    denied.allowed = false;
    denied.reason = "no_matching_rule".to_owned();
    denied.persona_id = String::new();
    denied.persona_label = String::new();
    denied.rule_id = String::new();
    denied.disclosed_keys.clear();
    let second = record(&svc, denied.clone()).await?;
    assert_eq!(second.seq, 2);

    let page = list(&svc, subject, 0, None).await?;
    assert_eq!(seqs(&page), [2, 1]);
    let newest = &page.decisions[0];
    assert_eq!(newest.prev_hash, first.hash);
    assert_eq!(newest.hash, second.hash);
    assert!(!newest.allowed);
    assert_eq!(newest.reason, "no_matching_rule");
    assert_eq!(newest.persona_id, "");
    assert_eq!(newest.persona_label, "");
    assert!(newest.disclosed_keys.is_empty());
    assert_eq!(newest.withheld_keys, ["phone"]);

    // hash(row 2) = SHA-256(hash(row 1) ‖ canonical(row 2)), absent ids as "".
    let prev: [u8; 32] = first.hash.as_slice().try_into()?;
    assert_eq!(second.hash, chain::row_hash(&prev, &committed(&denied)));

    let report = verify(&svc).await;
    assert!(report.ok);
    assert_eq!(report.length, 2);
    assert_eq!(report.head_hash, second.hash);
    Ok(())
}

#[sqlx::test(migrations = "./migrations")]
async fn nanosecond_timestamps_still_verify(pool: PgPool) -> anyhow::Result<()> {
    // Postgres keeps microseconds; the hash must commit to what is stored.
    let svc = AuditSvc::new(pool);
    let subject = ids::new_id();
    let mut d = decision(subject);
    d.decided_at.as_mut().expect("set").nanos = 123_456_789;
    record(&svc, d).await?;

    let page = list(&svc, subject, 0, None).await?;
    let stored = page.decisions[0].decided_at.as_ref().expect("decided_at");
    assert_eq!(stored.nanos, 123_456_000);
    assert!(verify(&svc).await.ok);
    Ok(())
}

#[sqlx::test(migrations = "./migrations")]
async fn list_pages_newest_first_by_seq(pool: PgPool) -> anyhow::Result<()> {
    let svc = AuditSvc::new(pool);
    let (alice, bob) = (ids::new_id(), ids::new_id());
    for owner in [alice, bob, alice, alice, bob, alice, alice] {
        record(&svc, decision(owner)).await?;
    }
    // alice holds seq 1, 3, 4, 6, 7; bob 2 and 5.

    let page = list(&svc, alice, 2, None).await?;
    assert_eq!(seqs(&page), [7, 6]);
    assert_eq!(page.next_before_seq, Some(6));
    let page = list(&svc, alice, 2, page.next_before_seq).await?;
    assert_eq!(seqs(&page), [4, 3]);
    assert_eq!(page.next_before_seq, Some(3));
    let page = list(&svc, alice, 2, page.next_before_seq).await?;
    assert_eq!(seqs(&page), [1]);
    assert_eq!(page.next_before_seq, None);

    let page = list(&svc, bob, 0, None).await?;
    assert_eq!(seqs(&page), [5, 2]);
    assert!(
        page.decisions
            .iter()
            .all(|d| d.subject_id == bob.to_string())
    );
    assert!(
        list(&svc, ids::new_id(), 0, None)
            .await?
            .decisions
            .is_empty()
    );

    let err = list_raw(&svc, "not-a-uuid").await.expect_err("rejected");
    assert_eq!(err.code(), Code::InvalidArgument);
    Ok(())
}

async fn list_raw(svc: &AuditSvc, subject_id: &str) -> Result<ListDecisionsResponse, Status> {
    svc.list_decisions(Request::new(ListDecisionsRequest {
        subject_id: subject_id.to_owned(),
        limit: 0,
        before_seq: None,
    }))
    .await
    .map(Response::into_inner)
}

#[sqlx::test(migrations = "./migrations")]
async fn limit_is_clamped_to_the_page_bounds(pool: PgPool) -> anyhow::Result<()> {
    let svc = AuditSvc::new(pool);
    let subject = ids::new_id();
    for _ in 0..201 {
        record(&svc, decision(subject)).await?;
    }

    // Above the maximum: 200 rows, and the cursor points at the one left.
    let page = list(&svc, subject, 1_000, None).await?;
    assert_eq!(page.decisions.len(), 200);
    assert_eq!(page.decisions[0].seq, 201);
    assert_eq!(page.next_before_seq, Some(2));
    let rest = list(&svc, subject, 1_000, page.next_before_seq).await?;
    assert_eq!(seqs(&rest), [1]);
    assert_eq!(rest.next_before_seq, None);

    // Unset: the default of 50.
    let page = list(&svc, subject, 0, None).await?;
    assert_eq!(page.decisions.len(), 50);
    assert_eq!(page.next_before_seq, Some(152));

    // Below the minimum: 1.
    let page = list(&svc, subject, -7, None).await?;
    assert_eq!(seqs(&page), [201]);
    assert_eq!(page.next_before_seq, Some(201));
    Ok(())
}

#[sqlx::test(migrations = "./migrations")]
async fn trigger_rejects_update_and_delete(pool: PgPool) -> anyhow::Result<()> {
    let svc = AuditSvc::new(pool.clone());
    record(&svc, decision(ids::new_id())).await?;

    for sql in [
        "update disclosure_events set allowed = false",
        "delete from disclosure_events",
    ] {
        let err = sqlx::query(sql).execute(&pool).await.expect_err(sql);
        assert!(
            matches!(&err, sqlx::Error::Database(db) if db.message().contains("append-only")),
            "{sql}: {err}"
        );
    }
    assert_eq!(row_count(&pool).await, 1);
    assert!(verify(&svc).await.ok);
    Ok(())
}

#[sqlx::test(migrations = "./migrations")]
async fn verify_reports_an_edited_row(pool: PgPool) -> anyhow::Result<()> {
    let svc = AuditSvc::new(pool.clone());
    let subject = ids::new_id();
    for _ in 0..3 {
        record(&svc, decision(subject)).await?;
    }
    assert!(verify(&svc).await.ok);

    without_trigger(
        &pool,
        "update disclosure_events set purpose = 'tampered' where seq = 2",
    )
    .await?;

    let report = verify(&svc).await;
    assert!(!report.ok);
    assert_eq!(report.broken_at_seq, Some(2));
    assert_eq!(report.length, 3);
    Ok(())
}

#[sqlx::test(migrations = "./migrations")]
async fn verify_reports_a_removed_row(pool: PgPool) -> anyhow::Result<()> {
    let svc = AuditSvc::new(pool.clone());
    let subject = ids::new_id();
    for _ in 0..3 {
        record(&svc, decision(subject)).await?;
    }

    without_trigger(&pool, "delete from disclosure_events where seq = 2").await?;

    // Row 3 still names row 2's hash as its predecessor.
    let report = verify(&svc).await;
    assert!(!report.ok);
    assert_eq!(report.broken_at_seq, Some(3));
    assert_eq!(report.length, 2);
    Ok(())
}

#[sqlx::test(migrations = "./migrations")]
async fn record_rejects_malformed_decisions(pool: PgPool) -> anyhow::Result<()> {
    let svc = AuditSvc::new(pool.clone());
    let subject = ids::new_id();

    let missing = svc
        .record(Request::new(RecordDecisionRequest { decision: None }))
        .await
        .expect_err("no decision");
    assert_eq!(missing.code(), Code::InvalidArgument);

    let mut bad_requester = decision(subject);
    bad_requester.requester_id = "not-a-uuid".to_owned();
    let mut bad_persona = decision(subject);
    bad_persona.persona_id = "p1".to_owned();
    let mut no_time = decision(subject);
    no_time.decided_at = None;
    let mut bad_time = decision(subject);
    bad_time.decided_at.as_mut().expect("set").seconds = i64::MAX;
    for (field, d) in [
        ("requester_id", bad_requester),
        ("persona_id", bad_persona),
        ("decided_at", no_time),
        ("decided_at", bad_time),
    ] {
        let err = record(&svc, d).await.expect_err(field);
        assert_eq!(err.code(), Code::InvalidArgument, "{field}: {err}");
        assert!(err.message().starts_with(field), "{field}: {err}");
    }
    assert_eq!(row_count(&pool).await, 0);
    Ok(())
}

#[sqlx::test(migrations = "./migrations")]
async fn unknown_subject_probes_are_recorded(pool: PgPool) -> anyhow::Result<()> {
    let svc = AuditSvc::new(pool);
    let mut probe = decision(Uuid::nil());
    probe.allowed = false;
    probe.reason = "unknown_subject".to_owned();
    probe.persona_id = String::new();
    probe.persona_label = String::new();
    probe.rule_id = String::new();
    probe.disclosed_keys.clear();
    probe.withheld_keys.clear();

    let res = record(&svc, probe).await?;
    assert_eq!(res.seq, 1);
    let page = list(&svc, Uuid::nil(), 0, None).await?;
    assert_eq!(page.decisions[0].subject_id, Uuid::nil().to_string());
    assert_eq!(page.decisions[0].reason, "unknown_subject");
    assert!(verify(&svc).await.ok);
    Ok(())
}

#[sqlx::test(migrations = "./migrations")]
async fn concurrent_records_keep_the_chain_intact(pool: PgPool) -> anyhow::Result<()> {
    let svc = AuditSvc::new(pool);
    let subject = ids::new_id();
    let mut tasks = tokio::task::JoinSet::new();
    for _ in 0..25 {
        let svc = svc.clone();
        tasks.spawn(async move { record(&svc, decision(subject)).await });
    }
    let mut seqs = Vec::new();
    while let Some(joined) = tasks.join_next().await {
        seqs.push(joined??.seq);
    }
    seqs.sort_unstable();
    assert_eq!(seqs, (1..=25).collect::<Vec<i64>>());

    let report = verify(&svc).await;
    assert!(report.ok, "chain broken at {:?}", report.broken_at_seq);
    assert_eq!(report.length, 25);
    Ok(())
}
