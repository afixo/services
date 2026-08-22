//! The hash chain. Pure: given the previous hash and a decision, compute the
//! row hash. `hash = SHA-256(prev_hash ‖ canonical(event))`, where canonical is
//! a fixed field order with length-prefixed strings, so that no two distinct
//! events can serialise to the same bytes.

use sha2::{Digest, Sha256};

pub const GENESIS: [u8; 32] = [0u8; 32];

/// The fields that are committed to by the hash — everything a verifier needs
/// and nothing that could legitimately differ between two faithful recordings
/// (so not `recorded_at`, which the consumer assigns).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Committed<'a> {
    pub event_id: &'a str,
    pub subject_id: &'a str,
    pub requester_id: &'a str,
    pub purpose: &'a str,
    pub allowed: bool,
    pub reason: &'a str,
    pub persona_id: &'a str,
    pub persona_label: &'a str,
    pub rule_id: &'a str,
    pub disclosed_keys: &'a [String],
    pub withheld_keys: &'a [String],
    /// Unix seconds + nanos of `decided_at`.
    pub decided_at: (i64, i32),
}

fn put(h: &mut Sha256, s: &str) {
    h.update((s.len() as u64).to_be_bytes());
    h.update(s.as_bytes());
}

fn put_list(h: &mut Sha256, items: &[String]) {
    h.update((items.len() as u64).to_be_bytes());
    for item in items {
        put(h, item);
    }
}

pub fn row_hash(prev_hash: &[u8; 32], c: &Committed<'_>) -> [u8; 32] {
    let mut h = Sha256::new();
    h.update(prev_hash);
    put(&mut h, c.event_id);
    put(&mut h, c.subject_id);
    put(&mut h, c.requester_id);
    put(&mut h, c.purpose);
    h.update([u8::from(c.allowed)]);
    put(&mut h, c.reason);
    put(&mut h, c.persona_id);
    put(&mut h, c.persona_label);
    put(&mut h, c.rule_id);
    put_list(&mut h, c.disclosed_keys);
    put_list(&mut h, c.withheld_keys);
    h.update(c.decided_at.0.to_be_bytes());
    h.update(c.decided_at.1.to_be_bytes());
    h.finalize().into()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sample(keys: &[String]) -> Committed<'_> {
        Committed {
            event_id: "e1",
            subject_id: "s1",
            requester_id: "r1",
            purpose: "shipping",
            allowed: true,
            reason: "rule_matched",
            persona_id: "p1",
            persona_label: "legal",
            rule_id: "rule1",
            disclosed_keys: keys,
            withheld_keys: &[],
            decided_at: (1_700_000_000, 0),
        }
    }

    #[test]
    fn deterministic_and_chained() {
        let keys = vec!["full_name".to_owned()];
        let c = sample(&keys);
        let a = row_hash(&GENESIS, &c);
        assert_eq!(a, row_hash(&GENESIS, &c));
        let b = row_hash(&a, &c);
        assert_ne!(a, b, "same event after a different prev_hash must differ");
    }

    #[test]
    fn length_prefix_prevents_boundary_ambiguity() {
        let k1 = vec!["ab".to_owned(), "c".to_owned()];
        let k2 = vec!["a".to_owned(), "bc".to_owned()];
        assert_ne!(
            row_hash(&GENESIS, &sample(&k1)),
            row_hash(&GENESIS, &sample(&k2))
        );
    }

    #[test]
    fn any_field_change_changes_the_hash() {
        let keys = vec![];
        let base = sample(&keys);
        let h = row_hash(&GENESIS, &base);
        let mut denied = base.clone();
        denied.allowed = false;
        assert_ne!(h, row_hash(&GENESIS, &denied));
    }
}
