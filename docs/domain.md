# Domain model and the disclosure decision

Condensed from `FINAL_REPORT.md` chapter 3, with the V2 changes marked.

## Vocabulary

| Term | Meaning | Owner |
|---|---|---|
| **Subject** | A human identity owner. Signs in with GitHub; identified publicly by a `handle` (`[a-z0-9-]{2,39}`). | identity |
| **Persona** | A named partial identity (`legal`, `work`, `social`…): a bundle of typed fields. Label unique per subject. | identity |
| **Field** | `key → value` with a **sensitivity** tier `0 public · 1 low · 2 medium · 3 high`. Key unique per persona. | identity |
| **Requester** | A registered machine client (`client_id` + secret, stored hashed). Owned by the subject who registered it. | auth |
| **Purpose** | One of a closed, seeded vocabulary: `social_display`, `professional`, `shipping`, `billing`, `age_verification`, `legal_kyc`, `support`. | policy |
| **Rule** | A grant: `(subject, requester?, purpose?) → persona, max_sensitivity, allow_keys?, priority`. `NULL` requester/purpose is a wildcard. | policy |
| **Decision** | The outcome of one disclosure request: allow (persona + released fields + withheld keys) or deny (reason). Always audited. | disclosure → audit |

## The decision (`crates/engine`)

Input: the candidate rules for `(subject, requester, purpose)` — policy's one
indexed query returns exact matches plus every applicable wildcard:

```sql
where subject_id = $1
  and (requester_id = $2 or requester_id is null)
  and (purpose      = $3 or purpose      is null)
```

1. **Rank by specificity.** `2·[requester named] + 1·[purpose named]`, so
   requester+purpose (3) > requester only (2) > purpose only (1) > global (0).
   *Who* asks is a harder-to-forge signal than *why*. Ties: higher `priority`,
   then newest rule (`created_seq`) — deterministic.
2. **Resolve the winner's persona** (identity). Missing, or belonging to another
   subject → deny.
3. **Filter fields.** Withhold when `field.sensitivity > max_sensitivity`, or
   when an allow-list exists and the key is not on it. Both mechanisms always
   apply; they are independent (ceiling scales with new fields, allow-list pins
   an exact contract).
4. **Deny by default** when nothing matched.
5. **Record** the decision (both outcomes), then answer.

Invariants (property-tested): released ⊆ persona fields; released ∩ withheld
= ∅ and released ∪ withheld = all keys; every released field is within the
ceiling and on the allow-list if one exists; no candidates ⇒ denied; the
winner is never beaten on specificity, nor on priority at equal specificity.

Withheld is reported as **key names only** — enough for an integrator to tell
"filtered" from "empty persona", without the values. (Deliberate middle
position, see report §4.3.)

## Two caller types, never the same principal

| | Subject | Requester |
|---|---|---|
| Authenticates with | GitHub OAuth | OAuth2 client-credentials |
| Holds | nothing — a sealed cookie at the edge | a bearer token (1 h) |
| Access token | opaque, 15 min | opaque, 1 h |
| Refresh | rotating refresh token, 7 d; reuse revokes the family | re-authenticate |
| Surface | console listener (`origin.afixo.io` via the Workers) | machine listener (`api.afixo.io`) |

## V2 changes relative to the report

| Report limitation (§5.4, §5.7) | V2 |
|---|---|
| Requester registration not scoped to an owner; anyone could rotate any secret | `requesters.owner_subject_id`; rotation requires ownership; directory listing exposes no secrets |
| `404` vs `403` on disclose let requesters enumerate handles | uniform `403` for every deny, including unknown subject |
| Audit is append-only by convention | append-only by trigger **and** hash-chained (`docs/audit.md`); `GET /v1/audit/verify` |
| Audit read is `LIMIT 200`, no paging | keyset pagination (`before=<seq>`) |
| Audit write in the request path | still in the request path — `audit.Record` gRPC awaited, idempotent on `event_id`; audit down ⇒ disclose fails closed (`docs/audit.md`) |
| Refresh tokens absent (KV sessions) | rotating refresh with reuse detection |
| 11 tests, engine only | engine unit + property tests; services get integration tests against real Postgres/NATS |
| Single scalar sensitivity; flat grant-only rules | **unchanged** — still scalar and grant-only (deliberate; fails safe). Category lattice and composition remain future work. |
| Purpose taken at face value | **unchanged** — declared purpose, auditable; RFC 9396 binding remains future work |

## Data ownership

```
auth      github_identities · oauth_states · sessions · requesters · requester_tokens
identity  subjects · personas · persona_fields
policy    purposes · disclosure_rules
audit     disclosure_events (append-only, hash-chained)
```

Cross-service ids are validated by RPC at write time (rule → persona via
identity, rule → requester via auth) so an invalid rule is never stored; at
read time a dangling reference is a deny, never an error.
