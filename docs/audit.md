# The audit path and the hash chain

## Synchronous, in the request path

```
disclosure.Disclose
  … resolve subject, select rule, fetch persona, run afixo-engine …
  audit.Record(DisclosureDecided)   ← awaited; failure ⇒ UNAVAILABLE ⇒ gateway 503
  respond to the gateway
```

The original implementation awaited the audit insert before writing the
response so that an unlogged disclosure was unreachable (FINAL_REPORT §4.3).
The rewrite keeps exactly that property with a synchronous gRPC call:
`disclosure` answers only after `audit.Record` succeeds, and if `audit` is
down the request **fails closed** — a `503`, never an unrecorded answer.

Consequences, stated rather than hidden:

- `audit` availability bounds `disclose` availability. Accepted: the log *is*
  the product's accountability story, and a disclosure nobody can audit is
  worse than a refused one.
- `Record` is idempotent on `event_id` so a timed-out call can be retried
  (one retry in `disclosure`, then give up with `UNAVAILABLE`).
- No message broker. An earlier draft used NATS JetStream with publish-ack;
  it was dropped as unnecessary machinery for this scale. If a second
  consumer of decisions ever appears (metrics, webhooks), reintroduce an
  event stream *downstream of* the audit table, not in front of it.

## The chain (`services/audit/src/chain.rs`)

Each row commits to the previous one:

```
prev_hash(genesis) = 32 × 0x00
hash = SHA-256( prev_hash ‖ canonical(decision) )
canonical = len-prefixed: event_id, subject_id, requester_id, purpose,
            allowed(1 byte), reason, persona_id, persona_label, rule_id,
            list(disclosed_keys), list(withheld_keys), decided_at(secs i64 BE, nanos i32 BE)
```

Length-prefixing makes the encoding injective (no `ab|c` vs `a|bc`).
`recorded_at` is not committed — it is assigned on insert and two faithful
recordings of the same decision may differ in it.

**Append** (one transaction): `select hash from disclosure_events order by seq
desc limit 1 for update` → compute → `insert … on conflict (event_id) do
nothing` → return the row's `seq`. The `audit` Deployment uses
`strategy: Recreate` so a surge pod never appends concurrently; the row lock
is the second line of defence.

**Verify** (`audit.VerifyChain` → `GET /v1/audit/verify`): walk `seq`
ascending, recompute, compare; report the first `broken_at_seq`. O(n); fine at
this scale, and a per-subject Merkle index is the obvious next step if not.

The table also carries an `update or delete` trigger that raises. A superuser
can drop it — which is exactly what the chain makes detectable after the fact.

## What is recorded

Key **names** only — never field values. The requester's `decision_id` is the
row's `event_id`, so a subject can find exactly the call a requester quotes.
Unknown handles are recorded too (`subject_id` = zero uuid,
`reason = unknown_subject`): the gateway still answers the uniform `403`, and
the log shows that a probe happened.
