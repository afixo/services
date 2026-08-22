//! JSON shapes of the REST API and their conversions from proto. Timestamps are
//! RFC 3339 strings; ids are uuid strings; sensitivity is the integer tier.

use afixo_common::time::from_proto;
use afixo_proto as pb;
use serde::{Deserialize, Serialize};
use time::format_description::well_known::Rfc3339;

fn rfc3339(t: Option<&prost_types::Timestamp>) -> Option<String> {
    t.and_then(from_proto).and_then(|d| d.format(&Rfc3339).ok())
}

#[derive(Debug, Serialize)]
pub struct Subject {
    pub id: String,
    pub handle: String,
    pub display_name: String,
    pub created_at: Option<String>,
}

impl From<pb::Subject> for Subject {
    fn from(s: pb::Subject) -> Self {
        Self {
            id: s.id,
            handle: s.handle,
            display_name: s.display_name,
            created_at: rfc3339(s.created_at.as_ref()),
        }
    }
}

#[derive(Debug, Serialize, Deserialize)]
pub struct Field {
    pub key: String,
    pub value: String,
    pub sensitivity: i32,
}

impl From<pb::Field> for Field {
    fn from(f: pb::Field) -> Self {
        Self {
            key: f.key,
            value: f.value,
            sensitivity: f.sensitivity,
        }
    }
}

#[derive(Debug, Serialize)]
pub struct Persona {
    pub id: String,
    pub subject_id: String,
    pub label: String,
    pub fields: Vec<Field>,
    pub created_at: Option<String>,
}

impl From<pb::Persona> for Persona {
    fn from(p: pb::Persona) -> Self {
        Self {
            id: p.id,
            subject_id: p.subject_id,
            label: p.label,
            fields: p.fields.into_iter().map(Into::into).collect(),
            created_at: rfc3339(p.created_at.as_ref()),
        }
    }
}

#[derive(Debug, Serialize)]
pub struct Requester {
    pub id: String,
    pub client_id: String,
    pub name: String,
    pub owner_subject_id: String,
    pub created_at: Option<String>,
    pub rotated_at: Option<String>,
}

impl From<pb::Requester> for Requester {
    fn from(r: pb::Requester) -> Self {
        Self {
            id: r.id,
            client_id: r.client_id,
            name: r.name,
            owner_subject_id: r.owner_subject_id,
            created_at: rfc3339(r.created_at.as_ref()),
            rotated_at: rfc3339(r.rotated_at.as_ref()),
        }
    }
}

#[derive(Debug, Serialize)]
pub struct CreatedRequester {
    #[serde(flatten)]
    pub requester: Requester,
    /// Shown exactly once.
    pub client_secret: String,
}

#[derive(Debug, Serialize)]
pub struct Purpose {
    pub name: String,
    pub description: String,
}

impl From<pb::Purpose> for Purpose {
    fn from(p: pb::Purpose) -> Self {
        Self {
            name: p.name,
            description: p.description,
        }
    }
}

#[derive(Debug, Serialize)]
pub struct Rule {
    pub id: String,
    pub subject_id: String,
    pub requester_id: Option<String>,
    pub purpose: Option<String>,
    pub persona_id: String,
    pub max_sensitivity: i32,
    pub allow_keys: Option<Vec<String>>,
    pub priority: i32,
    pub created_at: Option<String>,
}

impl From<pb::Rule> for Rule {
    fn from(r: pb::Rule) -> Self {
        Self {
            id: r.id,
            subject_id: r.subject_id,
            requester_id: r.requester_id,
            purpose: r.purpose,
            persona_id: r.persona_id,
            max_sensitivity: r.max_sensitivity,
            allow_keys: r.allow_list.map(|a| a.keys),
            priority: r.priority,
            created_at: rfc3339(r.created_at.as_ref()),
        }
    }
}

#[derive(Debug, Deserialize)]
pub struct CreateRule {
    pub requester_id: Option<String>,
    pub purpose: Option<String>,
    pub persona_id: String,
    pub max_sensitivity: i32,
    /// Omit or null for "ceiling only"; `[]` releases nothing.
    pub allow_keys: Option<Vec<String>>,
    #[serde(default)]
    pub priority: i32,
}

#[derive(Debug, Deserialize)]
pub struct CreatePersona {
    pub label: String,
}

#[derive(Debug, Deserialize)]
pub struct UpsertField {
    pub value: String,
    pub sensitivity: i32,
}

#[derive(Debug, Deserialize)]
pub struct CreateRequester {
    pub name: String,
}

#[derive(Debug, Deserialize)]
pub struct RefreshBody {
    pub refresh_token: String,
}

#[derive(Debug, Serialize)]
pub struct Session {
    pub access_token: String,
    pub refresh_token: String,
    pub access_expires_at: Option<String>,
    pub refresh_expires_at: Option<String>,
    pub subject: Option<Subject>,
    pub roles: Vec<String>,
    pub redirect_to: String,
}

impl From<pb::SessionTokens> for Session {
    fn from(s: pb::SessionTokens) -> Self {
        Self {
            access_token: s.access_token,
            refresh_token: s.refresh_token,
            access_expires_at: rfc3339(s.access_expires_at.as_ref()),
            refresh_expires_at: rfc3339(s.refresh_expires_at.as_ref()),
            subject: s.subject.map(Into::into),
            roles: s.roles,
            redirect_to: if s.redirect_to.is_empty() {
                "/app".to_owned()
            } else {
                s.redirect_to
            },
        }
    }
}

#[derive(Debug, Serialize)]
pub struct ClientToken {
    pub access_token: String,
    pub token_type: &'static str,
    pub expires_in: i64,
}

/// Body of `GET /v1/disclose/{handle}` — field names are the wire contract.
#[derive(Debug, Serialize)]
pub struct DiscloseBody {
    pub decision: &'static str,
    pub decision_id: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub persona: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub fields: Option<std::collections::BTreeMap<String, String>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub withheld: Option<Vec<String>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub reason: Option<String>,
}

#[derive(Debug, Serialize)]
pub struct AuditEvent {
    pub seq: i64,
    pub event_id: String,
    pub requester_id: String,
    pub purpose: String,
    pub allowed: bool,
    pub reason: String,
    pub persona_id: Option<String>,
    pub persona_label: Option<String>,
    pub disclosed_keys: Vec<String>,
    pub withheld_keys: Vec<String>,
    pub decided_at: Option<String>,
    pub recorded_at: Option<String>,
    pub hash: String,
    pub prev_hash: String,
}

impl From<pb::DecisionRecord> for AuditEvent {
    fn from(d: pb::DecisionRecord) -> Self {
        Self {
            seq: d.seq,
            event_id: d.event_id,
            requester_id: d.requester_id,
            purpose: d.purpose,
            allowed: d.allowed,
            reason: d.reason,
            persona_id: (!d.persona_id.is_empty()).then_some(d.persona_id),
            persona_label: (!d.persona_label.is_empty()).then_some(d.persona_label),
            disclosed_keys: d.disclosed_keys,
            withheld_keys: d.withheld_keys,
            decided_at: rfc3339(d.decided_at.as_ref()),
            recorded_at: rfc3339(d.recorded_at.as_ref()),
            hash: hex::encode(&d.hash),
            prev_hash: hex::encode(&d.prev_hash),
        }
    }
}

#[derive(Debug, Serialize)]
pub struct AuditPage {
    pub decisions: Vec<AuditEvent>,
    pub next_before: Option<i64>,
}

#[derive(Debug, Serialize)]
pub struct ChainVerification {
    pub ok: bool,
    pub length: i64,
    pub broken_at_seq: Option<i64>,
    pub head_hash: String,
}

#[derive(Debug, Deserialize)]
pub struct AuditQuery {
    pub limit: Option<i32>,
    pub before: Option<i64>,
}

#[derive(Debug, Deserialize)]
pub struct RequestersQuery {
    #[serde(default)]
    pub mine: bool,
}

#[derive(Debug, Deserialize)]
pub struct LoginQuery {
    pub redirect_to: Option<String>,
}

#[derive(Debug, Deserialize)]
pub struct CallbackQuery {
    pub code: String,
    pub state: String,
}

#[derive(Debug, Deserialize)]
pub struct DiscloseQuery {
    pub purpose: Option<String>,
}

#[derive(Debug, Deserialize)]
pub struct TokenForm {
    pub grant_type: Option<String>,
    pub client_id: Option<String>,
    pub client_secret: Option<String>,
}
