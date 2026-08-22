# afixo-engine

The disclosure decision as a pure function. No I/O, no async, no proto types —
only `uuid` and `serde`. This is the crate the rest of the platform exists to
serve; keep it that way.

```
select_rule(candidates) -> Option<CandidateRule>      rank: specificity ↓, priority ↓, created_seq ↓
filter_fields(rule, fields) -> (disclosed, withheld)   ceiling AND allow-list
decide(subject_id, winner, persona) -> Decision        deny by default; foreign/missing persona → deny
```

## Rules

- Grant-only. Never add a deny/negation rule type; the fail-safe argument of
  the report (§3.5) depends on it.
- Requester specificity outranks purpose specificity (2 vs 1). Do not "fix" this.
- `allow_keys: None` (ceiling only) and `Some(empty)` (release nothing) are
  different. Preserve the distinction in every conversion into this crate.
- `withheld` carries key names only.
- Any change here needs a unit test **and** must keep `tests/properties.rs`
  green. If you add a field to `CandidateRule`, extend the proptest strategy.

## Tests

`cargo test -p afixo-engine` — unit tests in `src/lib.rs`, property tests in
`tests/properties.rs` (subset/partition/ceiling/allow-list invariants, deny on
no candidates, winner maximality). Add a Criterion bench when the hot path is
measured (`criterion` is in the workspace deps list for that purpose).
