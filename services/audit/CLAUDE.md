# audit

Append-only, hash-chained record of every disclosure decision. One gRPC
service: `Record` (called synchronously by `disclosure`), `ListDecisions`,
`VerifyChain`. Status: **skeleton** — `chain.rs` (hashing) is complete and
unit-tested; the three RPCs answer `UNIMPLEMENTED`.

Owns (db `afixo_audit`): `disclosure_events` (+ an update/delete trigger that raises).

## Invariants

1. **Append only.** No RPC updates or deletes. The trigger enforces it against
   our own bugs; the chain makes a superuser's tampering detectable.
2. **`Record` is idempotent on `event_id`.** `disclosure` retries a timed-out
   call once; the second call must find the row and return its `seq`
   (`insert … on conflict (event_id) do nothing`, then select).
3. **One appender at a time.** Deployment `strategy: Recreate`; inside, the
   append transaction locks the chain head (`select hash … order by seq desc
   limit 1 for update`). Both, always.
4. The hash commits to the decision, not to `recorded_at` (`chain.rs`
   explains). Change the canonical encoding only with a new chain-version
   column — never silently.
5. Reads are scoped by `subject_id`; pagination is keyset (`before_seq`),
   newest first, `limit` clamped to `1..=200`.
6. `VerifyChain` walks everything in `seq` order. It is O(n) and fine today.
7. Availability matters more here than anywhere else: if this service is
   down, `disclose` fails closed for everyone (by design — see
   `docs/audit.md`). Keep it small and dependency-free; never call out to
   another service from `Record`.

## Implementation plan

1. `repo.rs`: `append(pool, &DisclosureDecided) -> Result<(i64, [u8; 32]), sqlx::Error>`
   (transaction: lock head → `chain::row_hash(prev, Committed{…})` → insert
   on conflict do nothing → select seq/hash by event_id → commit),
   `list(subject_id, limit, before_seq)`, `all_for_verify()` (ascending).
2. `server.rs`: `Record` (validate uuids with `ids::parse`, `decided_at`
   required; INVALID_ARGUMENT otherwise), `ListDecisions` (`next_before_seq`
   = last seq when the page is full), `VerifyChain` (recompute and compare).
3. Tests against `TEST_DATABASE_URL`: record twice with the same `event_id`
   → one row, same seq; chain verifies; a manual `update` fails (trigger);
   after disabling the trigger and editing a row, `VerifyChain` reports that seq.

Run: `make run-audit` (needs the db).
