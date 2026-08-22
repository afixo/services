//! Proto ⇄ engine conversions. The engine owns no proto types, so the two
//! worlds meet here and nowhere else.

use std::collections::BTreeSet;

use afixo_common::ids;
use afixo_engine::{CandidateRule, Field, Persona, Sensitivity};
use afixo_proto as pb;
use tonic::Status;

pub fn sensitivity(v: i32) -> Sensitivity {
    Sensitivity::from_i32_clamped(v)
}

pub fn candidate(rule: &pb::Rule) -> Result<CandidateRule, Status> {
    Ok(CandidateRule {
        id: ids::parse("rule.id", &rule.id)?,
        requester_id: ids::parse_opt("rule.requester_id", rule.requester_id.as_deref())?,
        purpose: rule.purpose.clone().filter(|p| !p.is_empty()),
        persona_id: ids::parse("rule.persona_id", &rule.persona_id)?,
        max_sensitivity: sensitivity(rule.max_sensitivity),
        allow_keys: rule
            .allow_list
            .as_ref()
            .map(|a| a.keys.iter().cloned().collect::<BTreeSet<_>>()),
        priority: rule.priority,
        created_seq: rule.created_seq,
    })
}

pub fn persona(p: pb::Persona) -> Result<Persona, Status> {
    Ok(Persona {
        id: ids::parse("persona.id", &p.id)?,
        subject_id: ids::parse("persona.subject_id", &p.subject_id)?,
        label: p.label,
        fields: p
            .fields
            .into_iter()
            .map(|f| Field {
                key: f.key,
                value: f.value,
                sensitivity: sensitivity(f.sensitivity),
            })
            .collect(),
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use uuid::Uuid;

    #[test]
    fn allow_list_presence_is_preserved() {
        let base = pb::Rule {
            id: Uuid::now_v7().to_string(),
            subject_id: Uuid::now_v7().to_string(),
            requester_id: None,
            purpose: Some(String::new()),
            persona_id: Uuid::now_v7().to_string(),
            max_sensitivity: 7,
            allow_list: None,
            priority: 1,
            created_at: None,
            created_seq: 9,
        };
        let none = candidate(&base).unwrap();
        assert_eq!(none.allow_keys, None);
        assert_eq!(none.purpose, None, "an empty purpose is a wildcard");
        assert_eq!(none.max_sensitivity, Sensitivity::High, "clamped");
        assert_eq!(none.created_seq, 9);

        let empty = candidate(&pb::Rule {
            allow_list: Some(pb::AllowList { keys: vec![] }),
            ..base.clone()
        })
        .unwrap();
        assert_eq!(
            empty.allow_keys,
            Some(BTreeSet::new()),
            "an empty allow-list releases nothing"
        );

        let bad = candidate(&pb::Rule {
            persona_id: "nope".into(),
            ..base
        });
        assert!(bad.is_err());
    }
}
