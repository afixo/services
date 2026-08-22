//! The orchestration end to end against in-process stand-ins for identity,
//! policy and audit (real tonic servers on ephemeral ports), through the
//! `DisclosureService` trait.

#![allow(clippy::unwrap_used, clippy::missing_panics_doc)]

#[path = "../src/convert.rs"]
mod convert;
#[path = "../src/server.rs"]
mod server;

use std::{
    collections::HashMap,
    sync::{
        Arc, Mutex,
        atomic::{AtomicUsize, Ordering},
    },
};

use afixo_proto::{
    AllowList, CreatePersonaRequest, CreateRuleRequest, DeleteFieldRequest, DeletePersonaRequest,
    DeletePersonaResponse, DeleteRuleRequest, DeleteRuleResponse, DiscloseRequest,
    DisclosureDecided, EnsureSubjectRequest, Field, GetPersonaRequest, GetSubjectRequest,
    ListCandidateRulesRequest, ListCandidateRulesResponse, ListDecisionsRequest,
    ListDecisionsResponse, ListPersonasRequest, ListPersonasResponse, ListPurposesRequest,
    ListPurposesResponse, ListRulesRequest, ListRulesResponse, Persona, RecordDecisionRequest,
    RecordDecisionResponse, ResolveSubjectRequest, Rule, Subject, UpsertFieldRequest,
    VerifyChainRequest, VerifyChainResponse,
    audit_service_server::{AuditService, AuditServiceServer},
    disclosure_service_server::DisclosureService,
    identity_service_server::{IdentityService, IdentityServiceServer},
    policy_service_server::{PolicyService, PolicyServiceServer},
};
use tokio::net::TcpListener;
use tokio_stream::wrappers::TcpListenerStream;
use tonic::{Code, Request, Response, Status, transport::Channel};
use uuid::Uuid;

use server::DisclosureSvc;

const PURPOSES: [&str; 3] = ["billing", "shipping", "professional"];

fn nope(what: &str) -> Status {
    Status::unimplemented(what.to_owned())
}

// ---------------------------------------------------------------------------

#[derive(Default)]
struct FakeIdentity {
    subjects: Mutex<Vec<Subject>>,
    personas: Mutex<Vec<Persona>>,
}

#[derive(Clone)]
struct IdentityStub(Arc<FakeIdentity>);

#[tonic::async_trait]
impl IdentityService for IdentityStub {
    async fn resolve_subject(
        &self,
        req: Request<ResolveSubjectRequest>,
    ) -> Result<Response<Subject>, Status> {
        self.0
            .subjects
            .lock()
            .unwrap()
            .iter()
            .find(|s| s.handle == req.get_ref().handle)
            .cloned()
            .map(Response::new)
            .ok_or_else(|| Status::not_found("subject"))
    }
    async fn get_persona(
        &self,
        req: Request<GetPersonaRequest>,
    ) -> Result<Response<Persona>, Status> {
        self.0
            .personas
            .lock()
            .unwrap()
            .iter()
            .find(|p| p.id == req.get_ref().persona_id)
            .cloned()
            .map(Response::new)
            .ok_or_else(|| Status::not_found("persona"))
    }
    async fn ensure_subject(
        &self,
        _: Request<EnsureSubjectRequest>,
    ) -> Result<Response<Subject>, Status> {
        Err(nope("EnsureSubject"))
    }
    async fn get_subject(
        &self,
        _: Request<GetSubjectRequest>,
    ) -> Result<Response<Subject>, Status> {
        Err(nope("GetSubject"))
    }
    async fn list_personas(
        &self,
        _: Request<ListPersonasRequest>,
    ) -> Result<Response<ListPersonasResponse>, Status> {
        Err(nope("ListPersonas"))
    }
    async fn create_persona(
        &self,
        _: Request<CreatePersonaRequest>,
    ) -> Result<Response<Persona>, Status> {
        Err(nope("CreatePersona"))
    }
    async fn delete_persona(
        &self,
        _: Request<DeletePersonaRequest>,
    ) -> Result<Response<DeletePersonaResponse>, Status> {
        Err(nope("DeletePersona"))
    }
    async fn upsert_field(
        &self,
        _: Request<UpsertFieldRequest>,
    ) -> Result<Response<Persona>, Status> {
        Err(nope("UpsertField"))
    }
    async fn delete_field(
        &self,
        _: Request<DeleteFieldRequest>,
    ) -> Result<Response<Persona>, Status> {
        Err(nope("DeleteField"))
    }
}

#[derive(Default)]
struct FakePolicy {
    rules: Mutex<Vec<Rule>>,
}

#[derive(Clone)]
struct PolicyStub(Arc<FakePolicy>);

#[tonic::async_trait]
impl PolicyService for PolicyStub {
    async fn list_candidate_rules(
        &self,
        req: Request<ListCandidateRulesRequest>,
    ) -> Result<Response<ListCandidateRulesResponse>, Status> {
        let req = req.get_ref();
        if !PURPOSES.contains(&req.purpose.as_str()) {
            return Err(Status::invalid_argument("purpose: not in the vocabulary"));
        }
        let rules = self
            .0
            .rules
            .lock()
            .unwrap()
            .iter()
            .filter(|r| r.subject_id == req.subject_id)
            .filter(|r| {
                r.requester_id
                    .as_deref()
                    .is_none_or(|x| x == req.requester_id)
            })
            .filter(|r| r.purpose.as_deref().is_none_or(|x| x == req.purpose))
            .cloned()
            .collect();
        Ok(Response::new(ListCandidateRulesResponse { rules }))
    }
    async fn list_purposes(
        &self,
        _: Request<ListPurposesRequest>,
    ) -> Result<Response<ListPurposesResponse>, Status> {
        Err(nope("ListPurposes"))
    }
    async fn list_rules(
        &self,
        _: Request<ListRulesRequest>,
    ) -> Result<Response<ListRulesResponse>, Status> {
        Err(nope("ListRules"))
    }
    async fn create_rule(&self, _: Request<CreateRuleRequest>) -> Result<Response<Rule>, Status> {
        Err(nope("CreateRule"))
    }
    async fn delete_rule(
        &self,
        _: Request<DeleteRuleRequest>,
    ) -> Result<Response<DeleteRuleResponse>, Status> {
        Err(nope("DeleteRule"))
    }
}

#[derive(Default)]
struct FakeAudit {
    recorded: Mutex<Vec<DisclosureDecided>>,
    attempts: AtomicUsize,
    /// how many calls to fail with UNAVAILABLE before succeeding
    fail_first: AtomicUsize,
}

#[derive(Clone)]
struct AuditStub(Arc<FakeAudit>);

#[tonic::async_trait]
impl AuditService for AuditStub {
    async fn record(
        &self,
        req: Request<RecordDecisionRequest>,
    ) -> Result<Response<RecordDecisionResponse>, Status> {
        self.0.attempts.fetch_add(1, Ordering::SeqCst);
        let remaining = self.0.fail_first.load(Ordering::SeqCst);
        if remaining > 0 {
            self.0.fail_first.store(remaining - 1, Ordering::SeqCst);
            return Err(Status::unavailable("audit db down"));
        }
        let decision = req.into_inner().decision.unwrap();
        let mut recorded = self.0.recorded.lock().unwrap();
        recorded.push(decision);
        let seq = i64::try_from(recorded.len()).unwrap();
        Ok(Response::new(RecordDecisionResponse {
            seq,
            hash: vec![1; 32],
        }))
    }
    async fn list_decisions(
        &self,
        _: Request<ListDecisionsRequest>,
    ) -> Result<Response<ListDecisionsResponse>, Status> {
        Err(nope("ListDecisions"))
    }
    async fn verify_chain(
        &self,
        _: Request<VerifyChainRequest>,
    ) -> Result<Response<VerifyChainResponse>, Status> {
        Err(nope("VerifyChain"))
    }
}

// ---------------------------------------------------------------------------

struct World {
    svc: DisclosureSvc,
    identity: Arc<FakeIdentity>,
    policy: Arc<FakePolicy>,
    audit: Arc<FakeAudit>,
    subject: Subject,
    requester: Uuid,
}

async fn serve(
    build: impl FnOnce(tonic::transport::Server) -> tonic::transport::server::Router,
) -> Channel {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    let router = build(tonic::transport::Server::builder());
    tokio::spawn(async move {
        router
            .serve_with_incoming(TcpListenerStream::new(listener))
            .await
            .unwrap();
    });
    Channel::from_shared(format!("http://{addr}"))
        .unwrap()
        .connect_lazy()
}

async fn world() -> World {
    let identity = Arc::new(FakeIdentity::default());
    let policy = Arc::new(FakePolicy::default());
    let audit = Arc::new(FakeAudit::default());
    let subject = Subject {
        id: Uuid::now_v7().to_string(),
        handle: "alice".into(),
        display_name: "Alice".into(),
        created_at: None,
    };
    identity.subjects.lock().unwrap().push(subject.clone());
    let svc = DisclosureSvc::new(
        serve(|mut s| {
            s.add_service(IdentityServiceServer::new(IdentityStub(Arc::clone(
                &identity,
            ))))
        })
        .await,
        serve(|mut s| s.add_service(PolicyServiceServer::new(PolicyStub(Arc::clone(&policy)))))
            .await,
        serve(|mut s| s.add_service(AuditServiceServer::new(AuditStub(Arc::clone(&audit))))).await,
    );
    World {
        svc,
        identity,
        policy,
        audit,
        subject,
        requester: Uuid::now_v7(),
    }
}

impl World {
    fn persona(&self, subject_id: &str, label: &str, fields: &[(&str, i32)]) -> Persona {
        let p = Persona {
            id: Uuid::now_v7().to_string(),
            subject_id: subject_id.to_owned(),
            label: label.into(),
            fields: fields
                .iter()
                .map(|(k, s)| Field {
                    key: (*k).to_owned(),
                    value: format!("v-{k}"),
                    sensitivity: *s,
                })
                .collect(),
            created_at: None,
        };
        self.identity.personas.lock().unwrap().push(p.clone());
        p
    }

    fn rule(
        &self,
        requester: Option<Uuid>,
        purpose: Option<&str>,
        persona: &Persona,
        max: i32,
        allow: Option<&[&str]>,
        priority: i32,
    ) -> Rule {
        let mut rules = self.policy.rules.lock().unwrap();
        let r = Rule {
            id: Uuid::now_v7().to_string(),
            subject_id: self.subject.id.clone(),
            requester_id: requester.map(|u| u.to_string()),
            purpose: purpose.map(str::to_owned),
            persona_id: persona.id.clone(),
            max_sensitivity: max,
            allow_list: allow.map(|keys| AllowList {
                keys: keys.iter().map(|k| (*k).to_owned()).collect(),
            }),
            priority,
            created_at: None,
            created_seq: i64::try_from(rules.len()).unwrap() + 1,
        };
        rules.push(r.clone());
        r
    }

    async fn disclose(
        &self,
        handle: &str,
        purpose: &str,
    ) -> Result<afixo_proto::DiscloseResponse, Status> {
        self.svc
            .disclose(Request::new(DiscloseRequest {
                subject_handle: handle.into(),
                requester_id: self.requester.to_string(),
                purpose: purpose.into(),
            }))
            .await
            .map(Response::into_inner)
    }

    fn recorded(&self) -> Vec<DisclosureDecided> {
        self.audit.recorded.lock().unwrap().clone()
    }
}

fn sorted(keys: &[String]) -> Vec<String> {
    let mut v = keys.to_vec();
    v.sort();
    v
}

// ---------------------------------------------------------------------------

#[tokio::test]
async fn allow_path_filters_and_records() {
    let w = world().await;
    let legal = w.persona(
        &w.subject.id,
        "legal",
        &[
            ("full_name", 1),
            ("email", 2),
            ("dob", 3),
            ("postal_address", 2),
        ],
    );
    w.rule(
        Some(w.requester),
        Some("billing"),
        &legal,
        3,
        Some(&["full_name", "email"]),
        0,
    );

    let r = w.disclose("Alice", "billing").await.unwrap();
    assert!(r.allowed);
    assert_eq!(r.reason, "rule_matched");
    assert_eq!(r.persona_label, "legal");
    let mut keys: Vec<_> = r.fields.keys().cloned().collect();
    keys.sort();
    assert_eq!(keys, ["email", "full_name"]);
    assert_eq!(r.fields["email"], "v-email");
    assert_eq!(sorted(&r.withheld_keys), ["dob", "postal_address"]);

    let events = w.recorded();
    assert_eq!(events.len(), 1);
    let e = &events[0];
    assert_eq!(
        e.event_id, r.decision_id,
        "decision_id is the audit event id"
    );
    assert!(e.allowed);
    assert_eq!(e.subject_id, w.subject.id);
    assert_eq!(e.requester_id, w.requester.to_string());
    assert_eq!(e.purpose, "billing");
    assert_eq!(sorted(&e.disclosed_keys), ["email", "full_name"]);
    assert_eq!(sorted(&e.withheld_keys), ["dob", "postal_address"]);
    assert_eq!(e.persona_label, "legal");
    assert!(e.decided_at.is_some());
}

#[tokio::test]
async fn no_rule_is_a_recorded_deny() {
    let w = world().await;
    let r = w.disclose("alice", "shipping").await.unwrap();
    assert!(!r.allowed);
    assert_eq!(r.reason, "no_matching_rule");
    assert!(r.fields.is_empty());
    let events = w.recorded();
    assert_eq!(events.len(), 1);
    assert!(!events[0].allowed);
    assert_eq!(events[0].reason, "no_matching_rule");
    assert_eq!(events[0].event_id, r.decision_id);
}

#[tokio::test]
async fn unknown_handle_is_a_recorded_deny_with_nil_subject() {
    let w = world().await;
    let r = w.disclose("nobody", "billing").await.unwrap();
    assert!(!r.allowed);
    assert_eq!(r.reason, "unknown_subject");
    let events = w.recorded();
    assert_eq!(events.len(), 1);
    assert_eq!(events[0].subject_id, Uuid::nil().to_string());
    assert_eq!(events[0].reason, "unknown_subject");
}

#[tokio::test]
async fn unknown_purpose_is_the_callers_error_and_nothing_is_recorded() {
    let w = world().await;
    let err = w.disclose("alice", "mind_reading").await.unwrap_err();
    assert_eq!(err.code(), Code::InvalidArgument);
    assert!(w.recorded().is_empty());
    let err = w.disclose("alice", "").await.unwrap_err();
    assert_eq!(err.code(), Code::InvalidArgument);
}

#[tokio::test]
async fn requester_specific_rule_beats_the_global_default() {
    let w = world().await;
    let work = w.persona(&w.subject.id, "work", &[("display_name", 0)]);
    let legal = w.persona(&w.subject.id, "legal", &[("full_name", 1)]);
    w.rule(None, None, &work, 3, None, 100);
    w.rule(Some(w.requester), None, &legal, 3, None, 0);
    let r = w.disclose("alice", "professional").await.unwrap();
    assert!(r.allowed);
    assert_eq!(r.persona_label, "legal");
}

#[tokio::test]
async fn foreign_or_missing_persona_is_a_deny() {
    let w = world().await;
    let someone_else = Uuid::now_v7().to_string();
    let foreign = w.persona(&someone_else, "theirs", &[("secret", 0)]);
    w.rule(None, Some("billing"), &foreign, 3, None, 0);
    let r = w.disclose("alice", "billing").await.unwrap();
    assert!(!r.allowed);
    assert_eq!(r.reason, "persona_foreign");
    assert!(r.fields.is_empty());

    let ghost = Persona {
        id: Uuid::now_v7().to_string(),
        subject_id: w.subject.id.clone(),
        label: "gone".into(),
        fields: vec![],
        created_at: None,
    };
    w.rule(Some(w.requester), Some("shipping"), &ghost, 3, None, 0);
    let r = w.disclose("alice", "shipping").await.unwrap();
    assert!(!r.allowed);
    assert_eq!(r.reason, "persona_missing");
    assert_eq!(w.recorded().len(), 2, "both denies were recorded");
}

#[tokio::test]
async fn audit_failure_is_retried_once_then_fails_closed() {
    let w = world().await;
    let legal = w.persona(&w.subject.id, "legal", &[("full_name", 1)]);
    w.rule(None, None, &legal, 3, None, 0);

    w.audit.fail_first.store(1, Ordering::SeqCst);
    let r = w.disclose("alice", "billing").await.unwrap();
    assert!(r.allowed, "one transient failure is absorbed by the retry");
    assert_eq!(w.audit.attempts.load(Ordering::SeqCst), 2);
    assert_eq!(w.recorded().len(), 1);

    w.audit.attempts.store(0, Ordering::SeqCst);
    w.audit.fail_first.store(2, Ordering::SeqCst);
    let err = w.disclose("alice", "billing").await.unwrap_err();
    assert_eq!(
        err.code(),
        Code::Unavailable,
        "unrecorded decisions are never answered"
    );
    assert_eq!(w.audit.attempts.load(Ordering::SeqCst), 2);
    assert_eq!(w.recorded().len(), 1, "nothing new was recorded");
}

#[tokio::test]
async fn values_never_reach_the_audit_event() {
    let w = world().await;
    let legal = w.persona(&w.subject.id, "legal", &[("full_name", 1)]);
    w.rule(None, None, &legal, 3, None, 0);
    w.disclose("alice", "billing").await.unwrap();
    let encoded = format!("{:?}", w.recorded());
    assert!(
        !encoded.contains("v-full_name"),
        "event carries keys only: {encoded}"
    );
    let _ = HashMap::<String, String>::new();
}
