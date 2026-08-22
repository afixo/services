# identity

Subjects, personas and sensitivity-tiered fields. Status: **implemented** —
all nine RPCs, with integration tests against Postgres.

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

## How it is built

```
src/lib.rs       exposes repo + server so tests can drive the service struct
src/main.rs      boot, migrate, serve (+ health)
src/server.rs    IdentityService impl: parsing + validation at the boundary
src/repo.rs      literal-SQL queries; ownership lives in the predicates
tests/identity.rs  integration tests (#[sqlx::test], fresh db per test)
migrations/      0001: subjects + personas + persona_fields
```

- **Ownership in SQL, not in Rust.** `DeletePersona` is `delete … where id = $1
  and subject_id = $2`; `UpsertField` is `insert … select id, … from personas
  where id = $1 and subject_id = $2 on conflict (persona_id, key) do update`;
  `DeleteField` is `delete … where key = $3 and persona_id in (select id from
  personas where id = $1 and subject_id = $2)`. `rows_affected == 0` is the only
  signal, and it becomes `NOT_FOUND` (`"persona"` / `"field"`) — never a
  distinct "not yours" answer. After a field mutation the persona is re-read
  scoped by `subject_id`, so the response is exactly what the owner would `Get`.
- **`GetPersona` scope is presence-based.** `subject_id: Some(s)` must parse and
  match (`… and ($2::uuid is null or subject_id = $2)`); `Some("")` is a
  malformed scope and fails closed with `INVALID_ARGUMENT`, it does not fall
  back to the unscoped read.
- **Fields are read `order by key collate "C"`** (byte order, independent of
  the database locale) and grouped in memory for `ListPersonas` (one query for
  the personas, one `= any($1)` for all their fields).
- **Validation (server.rs):** handle → trim, ASCII-lowercase,
  `ids::validate_handle`; label → trim, 1..=40 chars; key → `[A-Za-z0-9_.-]{1,64}`;
  value → ≤ 4096 chars; sensitivity → clamped. `ResolveSubject` answers
  `NOT_FOUND` for a malformed handle too (disclosure turns `NOT_FOUND` into the
  uniform deny; a handle that cannot exist must not answer differently).
- Errors go through `grpc::db_status` (duplicate label → `ALREADY_EXISTS`,
  unknown subject on create → `FAILED_PRECONDITION` from the FK) and
  `grpc::internal`. Logs carry `subject_id`/`persona_id` and outcomes only —
  no keys, no values; `FieldRow`'s `Debug` redacts `value`.

## Tests

`tests/identity.rs` uses `#[sqlx::test(migrations = "./migrations")]`: sqlx
creates a throw-away database per test on the server named by `DATABASE_URL`
(the user needs `CREATEDB`; the compose user is a superuser), applies the
migrations, and drops it when the test passes.

```sh
docker compose up -d postgres
DATABASE_URL=postgres://afixo:afixo@localhost:5432/afixo cargo test -p afixo-identity
```

Covered: `EnsureSubject` idempotency + handle validation; subject lookups;
persona create/list/get/delete; label validation and per-subject uniqueness;
field upsert, byte-ordered keys, clamp, validation limits, delete; cascade on
persona delete; cross-subject access is `NOT_FOUND` for every mutating RPC and
for `GetPersona` with a foreign `subject_id`, and leaves the owner's data
untouched; `GetPersona` without `subject_id` returns a foreign persona.

Note: sqlx's harness reads `DATABASE_URL`, not the workspace's
`TEST_DATABASE_URL`; the CI job must export `DATABASE_URL` as well for these
tests to run there.

Run: `make run-identity`.
