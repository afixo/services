//! Property tests: the invariants FINAL_REPORT §6.2 asked for, for any rule
//! set and any persona.
//!
//! * the released set is always a subset of the persona's fields
//! * released ∩ withheld = ∅ and released ∪ withheld = all keys
//! * every released field respects the ceiling and the allow-list
//! * no candidates ⇒ denied, always
//! * the winner is never beaten on specificity, and never on priority within
//!   the same specificity

use std::collections::BTreeSet;

use afixo_engine::{
    CandidateRule, Decision, Field, Persona, Sensitivity, decide, filter_fields, select_rule,
};
use proptest::prelude::*;
use uuid::Uuid;

fn sensitivity() -> impl Strategy<Value = Sensitivity> {
    prop_oneof![
        Just(Sensitivity::Public),
        Just(Sensitivity::Low),
        Just(Sensitivity::Medium),
        Just(Sensitivity::High)
    ]
}

fn key() -> impl Strategy<Value = String> {
    "[a-z_]{1,8}"
}

fn field() -> impl Strategy<Value = Field> {
    (key(), "[a-zA-Z0-9 ]{0,12}", sensitivity()).prop_map(|(key, value, sensitivity)| Field {
        key,
        value,
        sensitivity,
    })
}

/// A persona's fields: keys are unique (`(persona_id, key)` is the primary key).
fn unique_fields() -> impl Strategy<Value = Vec<Field>> {
    prop::collection::vec(field(), 0..12).prop_map(|mut fields| {
        let mut seen = BTreeSet::new();
        fields.retain(|f| seen.insert(f.key.clone()));
        fields
    })
}

fn persona(subject_id: Uuid, id: Uuid) -> impl Strategy<Value = Persona> {
    unique_fields().prop_map(move |fields| Persona {
        id,
        subject_id,
        label: "p".into(),
        fields,
    })
}

fn rule(persona_id: Uuid) -> impl Strategy<Value = CandidateRule> {
    (
        any::<bool>(),
        any::<bool>(),
        sensitivity(),
        prop::option::of(prop::collection::btree_set(key(), 0..6)),
        -10i32..10,
        0i64..1000,
    )
        .prop_map(
            move |(req, purp, max_sensitivity, allow_keys, priority, created_seq)| CandidateRule {
                id: Uuid::now_v7(),
                requester_id: req.then(Uuid::now_v7),
                purpose: purp.then(|| "shipping".to_owned()),
                persona_id,
                max_sensitivity,
                allow_keys,
                priority,
                created_seq,
            },
        )
}

proptest! {
    #[test]
    fn released_is_a_filtered_subset(rule in rule(Uuid::nil()), fields in unique_fields()) {
        let (disclosed, withheld) = filter_fields(&rule, &fields);
        let all: Vec<&str> = fields.iter().map(|f| f.key.as_str()).collect();
        let released: Vec<&str> = disclosed.iter().map(|f| f.key.as_str()).collect();
        let withheld: Vec<&str> = withheld.iter().map(String::as_str).collect();

        prop_assert_eq!(released.len() + withheld.len(), all.len());
        for f in &disclosed {
            prop_assert!(fields.contains(f));
            prop_assert!(f.sensitivity <= rule.max_sensitivity);
            if let Some(allow) = &rule.allow_keys {
                prop_assert!(allow.contains(&f.key));
            }
        }
        for k in &withheld {
            prop_assert!(all.contains(k));
            prop_assert!(!released.contains(k));
        }
    }

    #[test]
    fn no_candidates_means_denied(subject in any::<u128>().prop_map(Uuid::from_u128)) {
        let d = decide(subject, None, None);
        prop_assert!(!d.is_allowed());
    }

    #[test]
    fn winner_is_maximal(rules in prop::collection::vec(rule(Uuid::nil()), 1..8)) {
        let winner = select_rule(rules.clone()).unwrap();
        for r in &rules {
            prop_assert!(r.specificity() <= winner.specificity());
            if r.specificity() == winner.specificity() {
                prop_assert!(r.priority <= winner.priority);
            }
        }
    }

    #[test]
    fn decision_never_leaks_beyond_persona(
        rules in prop::collection::vec(rule(Uuid::nil()), 0..6),
        persona in persona(Uuid::from_u128(1), Uuid::nil()),
    ) {
        let subject = Uuid::from_u128(1);
        let winner = select_rule(rules);
        let all_keys: BTreeSet<&str> = persona.fields.iter().map(|f| f.key.as_str()).collect();
        match decide(subject, winner.clone(), Some(persona.clone())) {
            Decision::Allowed { disclosed, withheld, .. } => {
                prop_assert!(winner.is_some());
                for f in &disclosed {
                    prop_assert!(all_keys.contains(f.key.as_str()));
                }
                for k in &withheld {
                    prop_assert!(all_keys.contains(k.as_str()));
                }
            }
            Decision::Denied { .. } => prop_assert!(winner.is_none()),
        }
    }
}
