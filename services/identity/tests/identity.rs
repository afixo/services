//! Integration tests for identity-service against a real Postgres.
//!
//! `#[sqlx::test]` creates a fresh database per test from `DATABASE_URL`
//! (`docker compose up -d postgres`, then
//! `DATABASE_URL=postgres://afixo:afixo@localhost:5432/afixo cargo test -p afixo-identity`),
//! applies `migrations/`, and drops the database when the test passes. The
//! service struct is driven directly through the generated `IdentityService`
//! trait: no network, no gateway, so what is tested is exactly what the other
//! services see.

use afixo_identity::server::IdentitySvc;
use afixo_proto::{
    CreatePersonaRequest, DeleteFieldRequest, DeletePersonaRequest, DeletePersonaResponse,
    EnsureSubjectRequest, Field, GetPersonaRequest, GetSubjectRequest, ListPersonasRequest,
    Persona, ResolveSubjectRequest, Subject, UpsertFieldRequest,
    identity_service_server::IdentityService,
};
use sqlx::PgPool;
use tonic::{Code, Request, Response, Status};
use uuid::Uuid;

// ---------------------------------------------------------------------------
// helpers

fn assert_code<T: std::fmt::Debug>(res: Result<Response<T>, Status>, want: Code) {
    match res {
        Ok(ok) => panic!("expected {want:?}, got Ok({:?})", ok.into_inner()),
        Err(status) => assert_eq!(status.code(), want, "status: {status}"),
    }
}

async fn ensure(svc: &IdentitySvc, handle: &str, display_name: &str) -> Subject {
    svc.ensure_subject(Request::new(EnsureSubjectRequest {
        handle: handle.into(),
        display_name: display_name.into(),
    }))
    .await
    .expect("ensure subject")
    .into_inner()
}

async fn create(svc: &IdentitySvc, subject_id: &str, label: &str) -> Persona {
    svc.create_persona(Request::new(CreatePersonaRequest {
        subject_id: subject_id.into(),
        label: label.into(),
    }))
    .await
    .expect("create persona")
    .into_inner()
}

async fn try_create(
    svc: &IdentitySvc,
    subject_id: &str,
    label: &str,
) -> Result<Response<Persona>, Status> {
    svc.create_persona(Request::new(CreatePersonaRequest {
        subject_id: subject_id.into(),
        label: label.into(),
    }))
    .await
}

async fn get(
    svc: &IdentitySvc,
    persona_id: &str,
    subject_id: Option<&str>,
) -> Result<Response<Persona>, Status> {
    svc.get_persona(Request::new(GetPersonaRequest {
        persona_id: persona_id.into(),
        subject_id: subject_id.map(Into::into),
    }))
    .await
}

async fn list(svc: &IdentitySvc, subject_id: &str) -> Vec<Persona> {
    svc.list_personas(Request::new(ListPersonasRequest {
        subject_id: subject_id.into(),
    }))
    .await
    .expect("list personas")
    .into_inner()
    .personas
}

async fn delete_persona(
    svc: &IdentitySvc,
    subject_id: &str,
    persona_id: &str,
) -> Result<Response<DeletePersonaResponse>, Status> {
    svc.delete_persona(Request::new(DeletePersonaRequest {
        subject_id: subject_id.into(),
        persona_id: persona_id.into(),
    }))
    .await
}

async fn upsert(
    svc: &IdentitySvc,
    subject_id: &str,
    persona_id: &str,
    key: &str,
    value: &str,
    sensitivity: i32,
) -> Result<Response<Persona>, Status> {
    svc.upsert_field(Request::new(UpsertFieldRequest {
        subject_id: subject_id.into(),
        persona_id: persona_id.into(),
        field: Some(Field {
            key: key.into(),
            value: value.into(),
            sensitivity,
        }),
    }))
    .await
}

async fn delete_field(
    svc: &IdentitySvc,
    subject_id: &str,
    persona_id: &str,
    key: &str,
) -> Result<Response<Persona>, Status> {
    svc.delete_field(Request::new(DeleteFieldRequest {
        subject_id: subject_id.into(),
        persona_id: persona_id.into(),
        key: key.into(),
    }))
    .await
}

fn keys(persona: &Persona) -> Vec<&str> {
    persona.fields.iter().map(|f| f.key.as_str()).collect()
}

fn field<'a>(persona: &'a Persona, key: &str) -> &'a Field {
    persona
        .fields
        .iter()
        .find(|f| f.key == key)
        .unwrap_or_else(|| panic!("field {key} missing from {:?}", keys(persona)))
}

// ---------------------------------------------------------------------------
// subjects

#[sqlx::test(migrations = "./migrations")]
async fn ensure_subject_is_idempotent_on_handle(pool: PgPool) {
    let svc = IdentitySvc::new(pool);

    let first = ensure(&svc, "Alice-1", "Alice").await;
    assert_eq!(first.handle, "alice-1", "handle is lower-cased");
    assert_eq!(first.display_name, "Alice");
    assert!(first.created_at.is_some());

    let again = ensure(&svc, "  ALICE-1 ", "Alice Liddell").await;
    assert_eq!(again.id, first.id, "same subject on every login");
    assert_eq!(again.created_at, first.created_at);
    assert_eq!(
        again.display_name, "Alice Liddell",
        "display name refreshed"
    );

    let by_id = svc
        .get_subject(Request::new(GetSubjectRequest {
            subject_id: first.id.clone(),
        }))
        .await
        .expect("get subject")
        .into_inner();
    assert_eq!(by_id.handle, "alice-1");
    assert_eq!(by_id.display_name, "Alice Liddell");

    let by_handle = svc
        .resolve_subject(Request::new(ResolveSubjectRequest {
            handle: "Alice-1".into(),
        }))
        .await
        .expect("resolve subject")
        .into_inner();
    assert_eq!(by_handle.id, first.id);
}

#[sqlx::test(migrations = "./migrations")]
async fn ensure_subject_validates_the_handle(pool: PgPool) {
    let svc = IdentitySvc::new(pool);

    let too_long = "a".repeat(40);
    for bad in [
        "",
        "a",
        "-alice",
        "alice-",
        "al ice",
        "al_ice",
        "ali.ce",
        "alïce",
        too_long.as_str(),
    ] {
        let res = svc
            .ensure_subject(Request::new(EnsureSubjectRequest {
                handle: bad.into(),
                display_name: String::new(),
            }))
            .await;
        assert_code(res, Code::InvalidArgument);
    }

    let longest = "b".repeat(39);
    assert_eq!(ensure(&svc, &longest, "").await.handle, longest);
    assert_eq!(ensure(&svc, "c0", "").await.handle, "c0");
}

#[sqlx::test(migrations = "./migrations")]
async fn subject_lookups_miss_cleanly(pool: PgPool) {
    let svc = IdentitySvc::new(pool);
    ensure(&svc, "alice", "Alice").await;

    let res = svc
        .get_subject(Request::new(GetSubjectRequest {
            subject_id: Uuid::now_v7().to_string(),
        }))
        .await;
    assert_code(res, Code::NotFound);

    let res = svc
        .get_subject(Request::new(GetSubjectRequest {
            subject_id: "not-a-uuid".into(),
        }))
        .await;
    assert_code(res, Code::InvalidArgument);

    for absent in ["ghost", "Alice!", "", "-alice"] {
        let res = svc
            .resolve_subject(Request::new(ResolveSubjectRequest {
                handle: absent.into(),
            }))
            .await;
        assert_code(res, Code::NotFound);
    }
}

// ---------------------------------------------------------------------------
// personas

#[sqlx::test(migrations = "./migrations")]
async fn persona_lifecycle(pool: PgPool) {
    let svc = IdentitySvc::new(pool);
    let alice = ensure(&svc, "alice", "Alice").await;

    let legal = create(&svc, &alice.id, "legal").await;
    assert_eq!(legal.label, "legal");
    assert_eq!(legal.subject_id, alice.id);
    assert!(legal.fields.is_empty());
    assert!(legal.created_at.is_some());

    let listed = list(&svc, &alice.id).await;
    assert_eq!(listed.len(), 1);
    assert_eq!(listed[0].id, legal.id);

    let fetched = get(&svc, &legal.id, Some(&alice.id))
        .await
        .expect("get persona")
        .into_inner();
    assert_eq!(fetched, legal);

    delete_persona(&svc, &alice.id, &legal.id)
        .await
        .expect("delete persona");
    assert!(list(&svc, &alice.id).await.is_empty());
    assert_code(get(&svc, &legal.id, Some(&alice.id)).await, Code::NotFound);
    assert_code(get(&svc, &legal.id, None).await, Code::NotFound);
    assert_code(
        delete_persona(&svc, &alice.id, &legal.id).await,
        Code::NotFound,
    );
}

#[sqlx::test(migrations = "./migrations")]
async fn create_persona_validates_the_label(pool: PgPool) {
    let svc = IdentitySvc::new(pool);
    let alice = ensure(&svc, "alice", "Alice").await;

    let too_long = "x".repeat(41);
    for bad in ["", "   ", "\t\n", too_long.as_str()] {
        assert_code(
            try_create(&svc, &alice.id, bad).await,
            Code::InvalidArgument,
        );
    }
    assert_code(
        try_create(&svc, "not-a-uuid", "legal").await,
        Code::InvalidArgument,
    );

    assert_eq!(create(&svc, &alice.id, "  work  ").await.label, "work");
    let longest = "y".repeat(40);
    assert_eq!(create(&svc, &alice.id, &longest).await.label, longest);
}

#[sqlx::test(migrations = "./migrations")]
async fn duplicate_label_is_already_exists(pool: PgPool) {
    let svc = IdentitySvc::new(pool);
    let alice = ensure(&svc, "alice", "Alice").await;
    let bob = ensure(&svc, "bob", "Bob").await;

    create(&svc, &alice.id, "legal").await;
    assert_code(
        try_create(&svc, &alice.id, "legal").await,
        Code::AlreadyExists,
    );
    // Trimmed before the uniqueness check.
    assert_code(
        try_create(&svc, &alice.id, " legal ").await,
        Code::AlreadyExists,
    );

    // Labels are unique per subject, not globally.
    assert_eq!(create(&svc, &bob.id, "legal").await.label, "legal");
    assert_eq!(list(&svc, &alice.id).await.len(), 1);
    assert_eq!(list(&svc, &bob.id).await.len(), 1);
}

#[sqlx::test(migrations = "./migrations")]
async fn list_personas_carries_each_personas_fields(pool: PgPool) {
    let svc = IdentitySvc::new(pool);
    let alice = ensure(&svc, "alice", "Alice").await;

    let legal = create(&svc, &alice.id, "legal").await;
    let work = create(&svc, &alice.id, "work").await;
    let social = create(&svc, &alice.id, "social").await;
    upsert(&svc, &alice.id, &legal.id, "full_name", "Alice Liddell", 1)
        .await
        .expect("upsert");
    upsert(&svc, &alice.id, &legal.id, "dob", "1852-05-04", 3)
        .await
        .expect("upsert");
    upsert(&svc, &alice.id, &work.id, "title", "Adventurer", 0)
        .await
        .expect("upsert");

    let listed = list(&svc, &alice.id).await;
    let labels: Vec<&str> = listed.iter().map(|p| p.label.as_str()).collect();
    assert_eq!(labels, ["legal", "work", "social"], "creation order");
    assert_eq!(keys(&listed[0]), ["dob", "full_name"]);
    assert_eq!(field(&listed[0], "dob").sensitivity, 3);
    assert_eq!(keys(&listed[1]), ["title"]);
    assert_eq!(field(&listed[1], "title").value, "Adventurer");
    assert!(listed[2].fields.is_empty());
    assert_eq!(listed[2].id, social.id);
}

// ---------------------------------------------------------------------------
// fields

#[sqlx::test(migrations = "./migrations")]
async fn fields_are_upserted_and_ordered_by_key(pool: PgPool) {
    let svc = IdentitySvc::new(pool);
    let alice = ensure(&svc, "alice", "Alice").await;
    let legal = create(&svc, &alice.id, "legal").await;

    upsert(&svc, &alice.id, &legal.id, "zeta", "z", 2)
        .await
        .expect("upsert");
    upsert(&svc, &alice.id, &legal.id, "Alpha", "a", 0)
        .await
        .expect("upsert");
    let persona = upsert(&svc, &alice.id, &legal.id, "mid", "m", 1)
        .await
        .expect("upsert")
        .into_inner();
    assert_eq!(persona.id, legal.id);
    assert_eq!(persona.subject_id, alice.id);
    // Byte order, independent of the database locale.
    assert_eq!(keys(&persona), ["Alpha", "mid", "zeta"]);
    assert_eq!(field(&persona, "zeta").sensitivity, 2);

    let updated = upsert(&svc, &alice.id, &legal.id, "mid", "m2", 3)
        .await
        .expect("upsert")
        .into_inner();
    assert_eq!(keys(&updated), ["Alpha", "mid", "zeta"], "no duplicate key");
    assert_eq!(field(&updated, "mid").value, "m2");
    assert_eq!(field(&updated, "mid").sensitivity, 3);

    let fetched = get(&svc, &legal.id, Some(&alice.id))
        .await
        .expect("get persona")
        .into_inner();
    assert_eq!(fetched, updated, "mutation response equals a fresh read");
}

#[sqlx::test(migrations = "./migrations")]
async fn sensitivity_is_clamped_at_the_boundary(pool: PgPool) {
    let svc = IdentitySvc::new(pool);
    let alice = ensure(&svc, "alice", "Alice").await;
    let legal = create(&svc, &alice.id, "legal").await;

    for (given, stored) in [(99, 3), (4, 3), (3, 3), (2, 2), (1, 1), (0, 0), (-7, 0)] {
        let persona = upsert(&svc, &alice.id, &legal.id, "k", "v", given)
            .await
            .expect("upsert")
            .into_inner();
        assert_eq!(field(&persona, "k").sensitivity, stored, "given {given}");
    }
}

#[sqlx::test(migrations = "./migrations")]
async fn upsert_field_validates_its_input(pool: PgPool) {
    let svc = IdentitySvc::new(pool);
    let alice = ensure(&svc, "alice", "Alice").await;
    let legal = create(&svc, &alice.id, "legal").await;

    let res = svc
        .upsert_field(Request::new(UpsertFieldRequest {
            subject_id: alice.id.clone(),
            persona_id: legal.id.clone(),
            field: None,
        }))
        .await;
    assert_code(res, Code::InvalidArgument);

    let too_long = "k".repeat(65);
    for bad in ["", " ", "a b", "a/b", "é", "key\n", too_long.as_str()] {
        let res = upsert(&svc, &alice.id, &legal.id, bad, "v", 0).await;
        assert_code(res, Code::InvalidArgument);
    }
    let longest_key = "k".repeat(64);
    for good in ["A-z.0_9", "x", longest_key.as_str()] {
        upsert(&svc, &alice.id, &legal.id, good, "v", 0)
            .await
            .expect("valid key");
    }

    let too_long = "v".repeat(4097);
    let res = upsert(&svc, &alice.id, &legal.id, "v", &too_long, 0).await;
    assert_code(res, Code::InvalidArgument);
    let longest = "é".repeat(4096);
    let persona = upsert(&svc, &alice.id, &legal.id, "v", &longest, 0)
        .await
        .expect("4096 characters are allowed")
        .into_inner();
    assert_eq!(field(&persona, "v").value, longest);
    upsert(&svc, &alice.id, &legal.id, "empty", "", 0)
        .await
        .expect("an empty value is allowed");

    assert_code(
        upsert(&svc, "nope", &legal.id, "k", "v", 0).await,
        Code::InvalidArgument,
    );
    assert_code(
        upsert(&svc, &alice.id, "nope", "k", "v", 0).await,
        Code::InvalidArgument,
    );
}

#[sqlx::test(migrations = "./migrations")]
async fn delete_field_returns_the_persona_and_misses_cleanly(pool: PgPool) {
    let svc = IdentitySvc::new(pool);
    let alice = ensure(&svc, "alice", "Alice").await;
    let legal = create(&svc, &alice.id, "legal").await;
    upsert(&svc, &alice.id, &legal.id, "email", "a@example.com", 1)
        .await
        .expect("upsert");
    upsert(&svc, &alice.id, &legal.id, "phone", "+44", 2)
        .await
        .expect("upsert");

    let persona = delete_field(&svc, &alice.id, &legal.id, "email")
        .await
        .expect("delete field")
        .into_inner();
    assert_eq!(persona.id, legal.id);
    assert_eq!(keys(&persona), ["phone"]);

    assert_code(
        delete_field(&svc, &alice.id, &legal.id, "email").await,
        Code::NotFound,
    );
    assert_code(
        delete_field(&svc, &alice.id, &legal.id, "never").await,
        Code::NotFound,
    );
    assert_code(
        delete_field(&svc, &alice.id, &Uuid::now_v7().to_string(), "phone").await,
        Code::NotFound,
    );

    let fetched = get(&svc, &legal.id, Some(&alice.id))
        .await
        .expect("get persona")
        .into_inner();
    assert_eq!(keys(&fetched), ["phone"]);
}

#[sqlx::test(migrations = "./migrations")]
async fn delete_persona_removes_its_fields(pool: PgPool) {
    let svc = IdentitySvc::new(pool);
    let alice = ensure(&svc, "alice", "Alice").await;
    let legal = create(&svc, &alice.id, "legal").await;
    upsert(&svc, &alice.id, &legal.id, "email", "a@example.com", 1)
        .await
        .expect("upsert");

    delete_persona(&svc, &alice.id, &legal.id)
        .await
        .expect("delete persona");
    assert_code(get(&svc, &legal.id, None).await, Code::NotFound);
    assert_code(
        upsert(&svc, &alice.id, &legal.id, "email", "x", 0).await,
        Code::NotFound,
    );
    assert_code(
        delete_field(&svc, &alice.id, &legal.id, "email").await,
        Code::NotFound,
    );

    // The label is free again and the new persona starts empty.
    let fresh = create(&svc, &alice.id, "legal").await;
    assert_ne!(fresh.id, legal.id);
    assert!(fresh.fields.is_empty());
}

// ---------------------------------------------------------------------------
// ownership

#[sqlx::test(migrations = "./migrations")]
async fn cross_subject_access_is_not_found(pool: PgPool) {
    let svc = IdentitySvc::new(pool);
    let alice = ensure(&svc, "alice", "Alice").await;
    let bob = ensure(&svc, "bob", "Bob").await;
    let legal = create(&svc, &alice.id, "legal").await;
    upsert(&svc, &alice.id, &legal.id, "email", "a@example.com", 1)
        .await
        .expect("upsert");

    // Every mutating RPC, and the scoped read, answer NOT_FOUND for bob —
    // indistinguishable from a persona that does not exist.
    assert_code(get(&svc, &legal.id, Some(&bob.id)).await, Code::NotFound);
    assert_code(
        upsert(&svc, &bob.id, &legal.id, "email", "b@example.com", 0).await,
        Code::NotFound,
    );
    assert_code(
        upsert(&svc, &bob.id, &legal.id, "injected", "x", 0).await,
        Code::NotFound,
    );
    assert_code(
        delete_field(&svc, &bob.id, &legal.id, "email").await,
        Code::NotFound,
    );
    assert_code(
        delete_persona(&svc, &bob.id, &legal.id).await,
        Code::NotFound,
    );
    assert!(list(&svc, &bob.id).await.is_empty());

    // Nothing bob tried left a trace on alice's persona.
    let untouched = get(&svc, &legal.id, Some(&alice.id))
        .await
        .expect("alice still owns it")
        .into_inner();
    assert_eq!(keys(&untouched), ["email"]);
    assert_eq!(field(&untouched, "email").value, "a@example.com");
    assert_eq!(field(&untouched, "email").sensitivity, 1);
}

#[sqlx::test(migrations = "./migrations")]
async fn get_persona_without_subject_returns_any_persona(pool: PgPool) {
    let svc = IdentitySvc::new(pool);
    let alice = ensure(&svc, "alice", "Alice").await;
    let legal = create(&svc, &alice.id, "legal").await;
    upsert(&svc, &alice.id, &legal.id, "email", "a@example.com", 1)
        .await
        .expect("upsert");

    // disclosure reads without a scope and compares subject_id itself.
    let persona = get(&svc, &legal.id, None)
        .await
        .expect("unscoped read")
        .into_inner();
    assert_eq!(persona.subject_id, alice.id);
    assert_eq!(keys(&persona), ["email"]);

    assert_code(
        get(&svc, &Uuid::now_v7().to_string(), None).await,
        Code::NotFound,
    );
    // A present-but-empty scope is malformed, not "no scope": it fails closed.
    assert_code(get(&svc, &legal.id, Some("")).await, Code::InvalidArgument);
    assert_code(get(&svc, "nope", None).await, Code::InvalidArgument);
}
