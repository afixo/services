# audit

Append-only, hash-chained record of every disclosure decision. One gRPC
service: `Record` (called synchronously by `disclosure`), `ListDecisions`,
`VerifyChain`. Status: **implemented**, with integration tests against
Postgres (`tests/audit.rs`).

Owns (db `afixo_audit`): `disclosure_events` (+ an update/delete trigger that raises).

## Invariants

1. **Append only.** No RPC updates or deletes. The trigger enforces it against
   our own bugs; the chain makes a superuser's tampering detectable.
2. **`Record` is idempotent on `event_id`.** `disclosure` retries a timed-out
   call once; the second call must find the row and return its `seq`
   (`insert … on conflict (event_id) do nothing`, then select). A replay is
   not re-examined: the first write is the record.
3. **One appender at a time.** Deployment `strategy: Recreate`; inside, the
   append transaction takes a transaction-scoped advisory lock and then locks
   the chain head (`select hash … order by seq desc limit 1 for update`). All
   of it, always — see "Why the advisory lock" below.
4. The hash commits to the decision, not to `recorded_at` (`chain.rs`
   explains). Change the canonical encoding only with a new chain-version
   column — never silently. That includes how `repo::Decision` canonicalises
   (ids hyphenated lower-case, absent values as `""`, `decided_at` at
   microsecond precision).
5. Reads are scoped by `subject_id`; pagination is keyset (`before_seq`),
   newest first, `limit` clamped to `1..=200`.
6. `VerifyChain` walks everything in `seq` order. It is O(n) and fine today.
7. Availability matters more here than anywhere else: if this service is
   down, `disclose` fails closed for everyone (by design — see
   `docs/audit.md`). Keep it small and dependency-free; never call out to
   another service from `Record`.

## How it is built

```
src/lib.rs       the crate: chain · repo · server (so tests/ can drive the service struct)
src/main.rs      boot, migrate, serve (+ health)
src/chain.rs     canonical encoding + SHA-256 — pure, unit-tested
src/repo.rs      Decision (what a hash commits to), DecisionRow, append / list / all_ascending
src/server.rs    AuditService impl: validation, page limits, the verifier walk (unit-tested)
tests/audit.rs   integration tests, one fresh database per test
migrations/      0001: disclosure_events + the append-only trigger
```

- **`Record`**: `DisclosureDecided` → `repo::Decision` (`ids::parse`;
  `persona_id`/`rule_id` empty → `NULL`; `decided_at` required and in range;
  anything else is `INVALID_ARGUMENT` with the field name; the zero
  `subject_id` is allowed — unknown-handle probes). Then `repo::append`, one
  transaction: `pg_advisory_xact_lock` → read the head `for update` (none →
  genesis) → `Decision::row_hash` → `insert … on conflict (event_id) do
  nothing returning seq, hash` → on conflict, `select seq, hash … where
  event_id = $1`. Answers `{seq, hash}`.
- **Why the advisory lock**: the head row lock alone is not enough. Under
  READ COMMITTED a second appender that blocked on `for update` resumes with
  the snapshot its statement started with, never sees the row the first one
  inserted, and would chain onto the same predecessor (and an empty table has
  nothing to lock at all). The advisory lock serialises appenders; the next
  statement's snapshot then includes the previous append. The row lock stays
  as the second line of defence.
- **The hash commits to the stored row.** `Decision::row_hash` hashes ids in
  their hyphenated lower-case form and absent values as `""` — the form a row
  reads back in — and `decided_at` truncated to microseconds, because that is
  what `timestamptz` keeps. Append and verify go through the same function.
- `seq` may have gaps: a duplicate `event_id` still burns a sequence value.
  `VerifyChain.length` counts rows.
- **`ListDecisions`**: `limit` 0 → 50, otherwise clamped to `1..=200`;
  `seq < before_seq`, newest first; `next_before_seq` = the last row's `seq`
  when the page is full (a full last page yields one empty page after it).
- **`VerifyChain`**: loads every row (`fetch_all` — O(n) memory as well as
  time; switch to a `fetch` stream before that matters), recomputes each hash
  from the row and its predecessor's, reports the first row whose `prev_hash`
  or `hash` disagrees as `broken_at_seq`. `head_hash` is the stored head
  (genesis for an empty table). A broken chain is logged at `error`.

## Tests

```sh
docker compose up -d postgres
DATABASE_URL=postgres://afixo:afixo@localhost:5432/afixo cargo test -p afixo-audit
cargo test -p afixo-audit --lib     # chain, verifier walk, page limits: no database
```

`#[sqlx::test(migrations = "./migrations")]` reads `DATABASE_URL` (not
`TEST_DATABASE_URL`) and needs a role that may create databases; every test
runs in its own `_sqlx_test_…` database, dropped when it passes. Covered:
first row (`seq` 1, hash = the documented canonical encoding, `prev_hash` =
genesis), idempotent replay (same `seq`/hash, one row), chaining, nanosecond
timestamps, pagination and the `limit` clamp, the trigger raising on
`update`/`delete`, tamper detection after the trigger is disabled (an edited
row, a removed row), input validation, the zero-uuid probe, and 25 concurrent
`Record`s leaving the chain intact.

Run: `make run-audit` (needs the db).
