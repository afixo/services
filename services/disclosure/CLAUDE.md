# disclosure

The product's hot path: `GET /v1/disclose/:handle?purpose=` ends here. Owns
no data. Status: **skeleton** — `Disclose` answers `UNIMPLEMENTED`; the
engine it must call is finished (`crates/engine`); the three upstream clients
(identity, policy, audit) are wired at boot.

## The orchestration (fixed)

```
Disclose(subject_handle, requester_id, purpose)
  1. subject   = identity.ResolveSubject(handle)          NOT_FOUND → deny(unknown_subject), subject_id = zero uuid
  2. cands     = policy.ListCandidateRules(subject.id, requester_id, purpose)
                                                            INVALID_ARGUMENT (purpose) → propagate (gateway: 400)
  3. winner    = engine::select_rule(cands)                 None → deny(no_matching_rule)
  4. persona   = identity.GetPersona(winner.persona_id)     NOT_FOUND → None
  5. decision  = engine::decide(subject.id, winner, persona)
  6. event     = DisclosureDecided{ event_id = new_id(), …keys only… }
     audit.Record(event).await                              ← awaited; retry once on UNAVAILABLE/DEADLINE_EXCEEDED
                                                            still failing → UNAVAILABLE (gateway: 503); do NOT answer
  7. respond   DiscloseResponse{ decision_id = event_id, allowed, fields, withheld_keys, … }
```

Both allow and deny reach step 6. An unknown handle also reaches it (zero
`subject_id`, `reason = unknown_subject`): the gateway still returns the
uniform 403, and the audit row records that a probe happened.

## Invariants

1. **Record before respond.** Never `spawn` the audit call. Never answer when
   it fails. Never downgrade it to best-effort.
2. Field **values** never enter the audit event; only key names. The
   response is the only place values appear.
3. `decision_id` == `event_id` so a requester can quote it and the subject can
   find it in the audit log.
4. Upstream failures are `UNAVAILABLE`/`INTERNAL`, never a deny — a deny is a
   policy outcome, an error is not.
5. Timeouts: each upstream call ≤ 2 s (`tonic::Request::set_timeout`); the
   whole RPC ≤ 5 s.

## Implementation plan

1. `convert.rs`: proto `Rule` → `engine::CandidateRule` (uuid parse,
   `Sensitivity::from_i32_clamped`, `AllowList` presence → `Option<BTreeSet>`),
   proto `Persona` → `engine::Persona`, `engine::Decision` → response + event.
2. `server.rs::disclose` per the orchestration above.
3. Tests: unit tests for `convert.rs`; integration test with in-process mock
   `IdentityService`/`PolicyService`/`AuditService` tonic servers asserting:
   the allow path records exactly one event whose keys match the response;
   deny records; audit returning UNAVAILABLE twice ⇒ `UNAVAILABLE` and no
   response body; a retried `Record` is sent with the same `event_id`.
4. Later: a Criterion benchmark of `engine::decide` and a k6/`oha` load test
   against the machine listener (the report lists "performance unmeasured").

Run: `make run-disclosure` (needs identity, policy, audit).
