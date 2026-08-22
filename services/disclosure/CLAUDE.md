# disclosure

The product's hot path: `GET /v1/disclose/:handle?purpose=` ends here. Owns
no data. Status: **implemented** — orchestration, conversions and integration
tests against in-process identity/policy/audit stand-ins.

## The orchestration (`server.rs`)

```
Disclose(subject_handle, requester_id, purpose)
  1. subject   = identity.ResolveSubject(handle)          NOT_FOUND → deny(unknown_subject), subject_id = zero uuid
  2. cands     = policy.ListCandidateRules(subject.id, requester_id, purpose)
                                                            INVALID_ARGUMENT (purpose) → propagate (gateway: 400)
  3. winner    = engine::select_rule(cands)                 None → deny(no_matching_rule)
  4. persona   = identity.GetPersona(winner.persona_id)     NOT_FOUND → None
  5. decision  = engine::decide(subject.id, winner, persona)
  6. audit.Record(DisclosureDecided{ event_id = new_id(), …keys only… })
                                                            awaited; one retry on UNAVAILABLE/DEADLINE_EXCEEDED;
                                                            still failing → UNAVAILABLE (gateway: 503); do NOT answer
  7. respond   DiscloseResponse{ decision_id = event_id, allowed, fields, withheld_keys, … }
```

Both allow and deny reach step 6. An unknown handle also reaches it (zero
`subject_id`, `reason = unknown_subject`): the gateway still returns the
uniform 403, and the audit row records that a probe happened.

## Invariants

1. **Record before respond.** Never `spawn` the audit call. Never answer when
   it fails. Never downgrade it to best-effort.
2. Field **values** never enter the audit event; only key names (`outcome()`
   builds both views from one `Decision`). The response is the only place
   values appear.
3. `decision_id` == `event_id` so a requester can quote it and the subject can
   find it in the audit log.
4. Upstream failures are `UNAVAILABLE`/`INTERNAL`, never a deny — a deny is a
   policy outcome, an error is not. Only `identity.ResolveSubject` NOT_FOUND
   and `identity.GetPersona` NOT_FOUND are turned into denies.
5. Every upstream call has a 2 s deadline (`grpc-timeout` + a client-side timeout).

## Layout

```
src/main.rs       boot, three lazy channels (IDENTITY_URL, POLICY_URL, AUDIT_URL), serve (+ health)
src/convert.rs    proto Rule/Persona → engine CandidateRule/Persona (AllowList presence preserved)
src/server.rs     decide() (steps 1–5) · record() (step 6, retry once) · outcome() · DisclosureService impl
tests/disclose.rs real tonic servers on ephemeral ports standing in for identity/policy/audit
```

## Tests

`cargo test -p afixo-disclosure` (no database needed). Covered: allow path
filters by ceiling + allow-list and records keys only; no rule ⇒ recorded
deny; unknown handle ⇒ recorded deny with nil subject; unknown/empty purpose
⇒ INVALID_ARGUMENT and nothing recorded; requester-specific rule beats the
global default; foreign and missing persona ⇒ deny; audit failing once is
retried, failing twice fails closed with UNAVAILABLE and nothing recorded;
field values never reach the audit event.

Later: a Criterion benchmark of `engine::decide` and a load test of the
machine listener (the report lists "performance unmeasured").

Run: `make run-disclosure` (needs identity, policy, audit).
