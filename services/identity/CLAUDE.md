# identity

Subjects, personas and sensitivity-tiered fields. Status: **skeleton** —
schema and contract final, every RPC answers `UNIMPLEMENTED`.

Owns (db `afixo_identity`): `subjects`, `personas`, `persona_fields`. Called by
the gateway (subject CRUD), by `auth` (`EnsureSubject` on login), by `policy`
(`GetPersona` with `subject_id` to validate a rule) and by `disclosure`
(`ResolveSubject`, `GetPersona` without `subject_id`).

## Invariants

1. Every mutating RPC carries `subject_id` and the SQL predicate includes it
   (`where id = $1 and subject_id = $2`). A foreign persona is `NOT_FOUND`,
   indistinguishable from a missing one.
2. `GetPersona` with `subject_id` set filters by it; without, it returns the
   persona and the caller (`disclosure`) compares `persona.subject_id` itself —
   the engine denies on mismatch (`PersonaForeign`).
3. Sensitivity is clamped to `0..=3` at the boundary
   (`afixo_engine::Sensitivity::from_i32_clamped`) so the CHECK constraint can
   never be hit by a client value and an out-of-range tier can never be stored.
4. `EnsureSubject` is idempotent on `handle` (`insert … on conflict (handle) do
   update set display_name = excluded.display_name returning *`); handles are
   validated with `afixo_common::ids::validate_handle`.
5. `Persona` responses always include `fields` ordered by `key`, so the engine
   and the UI see a stable order.
6. Field values are personal data: never log them. (Encrypting values at rest
   with AES-GCM + AAD = `persona_id` is a reasonable V2 item; not in scope now.)

## Implementation plan

1. `repo.rs`: `SubjectRow`, `PersonaRow`, `FieldRow`; `get_persona_with_fields`
   as two queries (persona, then fields ordered by key).
2. RPCs in the order the dashboard needs them: `EnsureSubject`, `GetSubject`,
   `ResolveSubject` (by handle, `NOT_FOUND` if absent), `ListPersonas`
   (with fields — the list is small), `CreatePersona` (`ALREADY_EXISTS` on
   duplicate label via `grpc::db_status`), `GetPersona`, `DeletePersona`,
   `UpsertField` (`insert … on conflict (persona_id, key) do update`),
   `DeleteField`. Each `*Field` RPC returns the full persona.
3. Tests against `TEST_DATABASE_URL`: cross-subject access is `NOT_FOUND` for
   every mutating RPC (the test the report said it was missing), label
   uniqueness, clamp, `EnsureSubject` idempotency.

Run: `make run-identity`.
