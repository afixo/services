# policy

The purpose vocabulary and disclosure rules. Status: **implemented**
(`ListPurposes`, `ListRules`, `CreateRule`, `DeleteRule`, `ListCandidateRules`);
needs integration tests.

Owns (db `afixo_policy`): `purposes` (seeded by migration), `disclosure_rules`.
Calls `identity.GetPersona` and `auth.GetRequester` to validate a rule at write
time (those RPCs are still skeletons, so `CreateRule` currently surfaces
`UNIMPLEMENTED` from them).

## Invariants

1. **Store and select, never rank.** `ListCandidateRules` is the report's one
   indexed query (exact + wildcards); ranking and filtering are `afixo-engine`,
   run by `disclosure`. Do not add `ORDER BY CASE …` specificity logic here.
2. Adding a purpose is a migration, not an RPC. The vocabulary is closed so
   purpose limitation is checkable.
3. An invalid rule is never stored: purpose must exist; persona must belong to
   the subject (identity, `subject_id` set); requester must exist (auth).
   `INVALID_ARGUMENT` with a field-prefixed message on failure.
4. `allow_keys`: `NULL` = ceiling only, `'{}'` = release nothing — the proto
   carries this as presence of `AllowList`. Keep the distinction through every layer.
5. Deleting a rule includes `subject_id` in the predicate; a foreign rule id is
   `NOT_FOUND`.
6. `created_seq` (bigserial) is the engine's final tie-break; never reuse or
   reset it.

## Layout

```
src/main.rs      boot, migrate, serve (+ health)
src/server.rs    PolicyService impl
src/repo.rs      literal-SQL queries (sqlx 0.9 forbids dynamic SQL strings)
migrations/      0001: purposes (seeded) + disclosure_rules + indexes
```

## Next

- Integration tests (`TEST_DATABASE_URL`): candidate selection returns exact +
  both wildcards + global and nothing from other subjects; unknown purpose is
  `INVALID_ARGUMENT`; delete is owner-scoped.
- `UpdateRule` is intentionally absent (rules are small; delete + create keeps
  `created_seq` semantics honest). Add only if the UI truly needs it.

Run: `make run-policy`; `grpcurl -plaintext localhost:50053 afixo.v1.PolicyService/ListPurposes`
(reflection is not enabled — pass `-proto proto/afixo/v1/policy.proto -import-path proto`).
