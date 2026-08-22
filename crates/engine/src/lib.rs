//! # afixo-engine
//!
//! The disclosure decision as one pure function. Everything the rest of the
//! platform exists to serve lives here, and it has no I/O so it can be tested
//! exhaustively (unit + property tests) without a database or a network.
//!
//! Given the candidate rules for `(subject, requester, purpose)` — already
//! narrowed by policy-service's indexed query to exact matches plus applicable
//! wildcards — and the persona the winning rule points at:
//!
//! 1. **Rank by specificity.** A rule scores 2 for naming a requester and 1
//!    for naming a purpose. *Who* is asking is a stronger, harder-to-forge
//!    signal than *why*, so requester specificity outranks purpose. Ties break
//!    on explicit `priority`, then on newest rule.
//! 2. **Filter fields.** Withhold any field whose sensitivity exceeds the
//!    rule's ceiling, or — when an allow-list is present — whose key is not on
//!    it. The two mechanisms are independent and both always apply.
//! 3. **Deny by default.** No candidates, a persona that is missing, or a
//!    persona that belongs to a different subject → `Denied`.
//!
//! Rules only grant. There is no deny rule and no negation; the worst outcome
//! of a misunderstood rule is under-disclosure, which fails safe.

use std::collections::BTreeSet;

use serde::{Deserialize, Serialize};
use uuid::Uuid;

/// The four-point sensitivity tier. Ordering is the contract.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[repr(u8)]
pub enum Sensitivity {
    Public = 0,
    Low = 1,
    Medium = 2,
    High = 3,
}

impl Sensitivity {
    /// Clamp any integer into the tier range — the boundary must never store
    /// an out-of-range tier that could later compare oddly against a ceiling.
    pub fn from_i32_clamped(v: i32) -> Self {
        match v {
            i32::MIN..=0 => Self::Public,
            1 => Self::Low,
            2 => Self::Medium,
            _ => Self::High,
        }
    }

    pub fn as_i16(self) -> i16 {
        self as i16
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Field {
    pub key: String,
    pub value: String,
    pub sensitivity: Sensitivity,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Persona {
    pub id: Uuid,
    pub subject_id: Uuid,
    pub label: String,
    pub fields: Vec<Field>,
}

/// A candidate rule as returned by policy-service. `requester_id`/`purpose`
/// `None` means the rule is a wildcard on that axis.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CandidateRule {
    pub id: Uuid,
    pub requester_id: Option<Uuid>,
    pub purpose: Option<String>,
    pub persona_id: Uuid,
    pub max_sensitivity: Sensitivity,
    /// `None` = ceiling only. `Some(empty)` = nothing is released.
    pub allow_keys: Option<BTreeSet<String>>,
    pub priority: i32,
    /// Creation order; higher = newer. Used only as the final tie-break so the
    /// decision is deterministic.
    pub created_seq: i64,
}

impl CandidateRule {
    /// 3 = requester+purpose, 2 = requester only, 1 = purpose only, 0 = global default.
    pub fn specificity(&self) -> u8 {
        u8::from(self.requester_id.is_some()) * 2 + u8::from(self.purpose.is_some())
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum DenyReason {
    /// No rule matched `(subject, requester, purpose)`.
    NoMatchingRule,
    /// The winning rule points at a persona that no longer exists.
    PersonaMissing,
    /// The persona exists but belongs to a different subject (must never
    /// happen if policy-service validated at write time; denied anyway).
    PersonaForeign,
}

impl DenyReason {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::NoMatchingRule => "no_matching_rule",
            Self::PersonaMissing => "persona_missing",
            Self::PersonaForeign => "persona_foreign",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum Decision {
    Allowed {
        rule_id: Uuid,
        persona_id: Uuid,
        persona_label: String,
        disclosed: Vec<Field>,
        /// Keys filtered out — names only, so a consumer can tell "filtered"
        /// from "empty persona" without learning the values.
        withheld: Vec<String>,
    },
    Denied {
        reason: DenyReason,
        /// Set when a rule matched but its persona could not be used.
        rule_id: Option<Uuid>,
    },
}

impl Decision {
    pub fn is_allowed(&self) -> bool {
        matches!(self, Self::Allowed { .. })
    }
}

/// Step 1: pick the winning rule. Returns `None` when there are no candidates.
pub fn select_rule(mut candidates: Vec<CandidateRule>) -> Option<CandidateRule> {
    candidates.sort_by(|a, b| {
        b.specificity()
            .cmp(&a.specificity())
            .then_with(|| b.priority.cmp(&a.priority))
            .then_with(|| b.created_seq.cmp(&a.created_seq))
    });
    candidates.into_iter().next()
}

/// Step 2: partition a persona's fields into released and withheld.
pub fn filter_fields(rule: &CandidateRule, fields: &[Field]) -> (Vec<Field>, Vec<String>) {
    let mut disclosed = Vec::with_capacity(fields.len());
    let mut withheld = Vec::new();
    for f in fields {
        let over_ceiling = f.sensitivity > rule.max_sensitivity;
        let not_allowed = rule
            .allow_keys
            .as_ref()
            .is_some_and(|allow| !allow.contains(&f.key));
        if over_ceiling || not_allowed {
            withheld.push(f.key.clone());
        } else {
            disclosed.push(f.clone());
        }
    }
    (disclosed, withheld)
}

/// The whole decision. `persona` is the persona the caller fetched for the
/// winning rule (use [`select_rule`] first to learn which id to fetch), or
/// `None` if it no longer exists.
pub fn decide(
    subject_id: Uuid,
    winner: Option<CandidateRule>,
    persona: Option<Persona>,
) -> Decision {
    let Some(rule) = winner else {
        return Decision::Denied {
            reason: DenyReason::NoMatchingRule,
            rule_id: None,
        };
    };
    let Some(persona) = persona else {
        return Decision::Denied {
            reason: DenyReason::PersonaMissing,
            rule_id: Some(rule.id),
        };
    };
    if persona.subject_id != subject_id || persona.id != rule.persona_id {
        return Decision::Denied {
            reason: DenyReason::PersonaForeign,
            rule_id: Some(rule.id),
        };
    }
    let (disclosed, withheld) = filter_fields(&rule, &persona.fields);
    Decision::Allowed {
        rule_id: rule.id,
        persona_id: persona.id,
        persona_label: persona.label,
        disclosed,
        withheld,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn rule(req: bool, purpose: bool, priority: i32, seq: i64) -> CandidateRule {
        CandidateRule {
            id: Uuid::now_v7(),
            requester_id: req.then(Uuid::now_v7),
            purpose: purpose.then(|| "shipping".to_owned()),
            persona_id: Uuid::now_v7(),
            max_sensitivity: Sensitivity::High,
            allow_keys: None,
            priority,
            created_seq: seq,
        }
    }

    fn field(key: &str, s: Sensitivity) -> Field {
        Field {
            key: key.into(),
            value: format!("v-{key}"),
            sensitivity: s,
        }
    }

    #[test]
    fn specificity_scores() {
        assert_eq!(rule(true, true, 0, 0).specificity(), 3);
        assert_eq!(rule(true, false, 0, 0).specificity(), 2);
        assert_eq!(rule(false, true, 0, 0).specificity(), 1);
        assert_eq!(rule(false, false, 0, 0).specificity(), 0);
    }

    #[test]
    fn requester_specific_beats_purpose_specific_regardless_of_priority() {
        let purpose_only = rule(false, true, 100, 1);
        let requester_only = rule(true, false, 0, 0);
        let winner = select_rule(vec![purpose_only, requester_only.clone()]).unwrap();
        assert_eq!(winner.id, requester_only.id);
    }

    #[test]
    fn ties_break_on_priority_then_newest() {
        let a = rule(false, false, 1, 1);
        let b = rule(false, false, 5, 0);
        assert_eq!(select_rule(vec![a.clone(), b.clone()]).unwrap().id, b.id);
        let c = rule(false, false, 5, 2);
        assert_eq!(select_rule(vec![b, c.clone()]).unwrap().id, c.id);
    }

    #[test]
    fn ceiling_withholds_high_fields() {
        let mut r = rule(false, true, 0, 0);
        r.max_sensitivity = Sensitivity::Medium;
        let fields = [
            field("name", Sensitivity::Low),
            field("dob", Sensitivity::High),
        ];
        let (d, w) = filter_fields(&r, &fields);
        assert_eq!(d.len(), 1);
        assert_eq!(d[0].key, "name");
        assert_eq!(w, vec!["dob"]);
    }

    #[test]
    fn allow_list_narrows_even_under_a_permissive_ceiling() {
        let mut r = rule(false, true, 0, 0);
        r.allow_keys = Some(
            ["full_name", "email"]
                .into_iter()
                .map(String::from)
                .collect(),
        );
        let fields = [
            field("full_name", Sensitivity::Low),
            field("email", Sensitivity::Medium),
            field("dob", Sensitivity::High),
            field("postal_address", Sensitivity::Medium),
        ];
        let (d, w) = filter_fields(&r, &fields);
        let keys: Vec<_> = d.iter().map(|f| f.key.as_str()).collect();
        assert_eq!(keys, ["full_name", "email"]);
        assert_eq!(w, ["dob", "postal_address"]);
    }

    #[test]
    fn empty_allow_list_releases_nothing() {
        let mut r = rule(false, true, 0, 0);
        r.allow_keys = Some(BTreeSet::new());
        let (d, w) = filter_fields(&r, &[field("name", Sensitivity::Public)]);
        assert!(d.is_empty());
        assert_eq!(w, ["name"]);
    }

    #[test]
    fn deny_by_default_paths() {
        let subject = Uuid::now_v7();
        assert_eq!(
            decide(subject, None, None),
            Decision::Denied {
                reason: DenyReason::NoMatchingRule,
                rule_id: None
            }
        );

        let r = rule(true, true, 0, 0);
        assert_eq!(
            decide(subject, Some(r.clone()), None),
            Decision::Denied {
                reason: DenyReason::PersonaMissing,
                rule_id: Some(r.id)
            }
        );

        let foreign = Persona {
            id: r.persona_id,
            subject_id: Uuid::now_v7(),
            label: "x".into(),
            fields: vec![],
        };
        assert_eq!(
            decide(subject, Some(r.clone()), Some(foreign)),
            Decision::Denied {
                reason: DenyReason::PersonaForeign,
                rule_id: Some(r.id)
            }
        );

        let wrong_persona = Persona {
            id: Uuid::now_v7(),
            subject_id: subject,
            label: "x".into(),
            fields: vec![],
        };
        assert!(!decide(subject, Some(r), Some(wrong_persona)).is_allowed());
    }

    #[test]
    fn allowed_path_reports_label_and_partitions() {
        let subject = Uuid::now_v7();
        let mut r = rule(true, true, 0, 0);
        r.max_sensitivity = Sensitivity::Low;
        let persona = Persona {
            id: r.persona_id,
            subject_id: subject,
            label: "legal".into(),
            fields: vec![
                field("full_name", Sensitivity::Low),
                field("dob", Sensitivity::High),
            ],
        };
        match decide(subject, Some(r.clone()), Some(persona)) {
            Decision::Allowed {
                rule_id,
                persona_label,
                disclosed,
                withheld,
                ..
            } => {
                assert_eq!(rule_id, r.id);
                assert_eq!(persona_label, "legal");
                assert_eq!(disclosed.len(), 1);
                assert_eq!(withheld, ["dob"]);
            }
            Decision::Denied { .. } => panic!("expected allow"),
        }
    }

    #[test]
    fn sensitivity_clamps() {
        assert_eq!(Sensitivity::from_i32_clamped(-7), Sensitivity::Public);
        assert_eq!(Sensitivity::from_i32_clamped(3), Sensitivity::High);
        assert_eq!(Sensitivity::from_i32_clamped(99), Sensitivity::High);
    }
}
