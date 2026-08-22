# REST API (gateway)

Canonical for the three repos. Two listeners; both are reached only through
the Workers (`afixo-web` → `afixo-api` → tunnel):

- **console listener `:8080`** — `origin.afixo.io`; the browser sees it as
  `https://afixo.io/api/v1/…` (the Workers add `/api`, the api Worker swaps
  the session cookie for a bearer).
- **machine listener `:8081`** — `origin-api.afixo.io`; clients see it as
  `https://api.afixo.io/…` (same paths, no `/api` prefix; their own bearer is
  passed through).

Errors everywhere: `{"error":"<snake_code>","message":"<human>"}`. OAuth codes
where the OAuth spec applies (`invalid_request`, `invalid_client`,
`invalid_token`, `unsupported_grant_type`). `401` responses carry
`WWW-Authenticate`.

Timestamps are RFC 3339; ids are uuids; `sensitivity` is `0..3`.

## Console listener — subject traffic

Auth = `Authorization: Bearer <subject access token>` (injected by `afixo-api`).

| Method | Path | Auth | Body / query | Response |
|---|---|---|---|---|
| GET | `/v1/health` | — | | `{status, service, version}` |
| GET | `/v1/purposes` | — | | `[{name, description}]` |
| GET | `/v1/auth/github/login` | — | `?redirect_to=/app/…` | `302` to GitHub |
| GET | `/v1/auth/github/callback` | — | `?code&state` | `Session` (below) — the Worker seals it |
| POST | `/v1/auth/refresh` | — | `{refresh_token}` | `Session` |
| POST | `/v1/auth/logout` | subject | | `204` |
| GET | `/v1/auth/me` | subject | | `Subject` |
| GET | `/v1/personas` | subject | | `[Persona]` |
| POST | `/v1/personas` | subject | `{label}` | `201 Persona` |
| GET | `/v1/personas/{id}` | subject | | `Persona` |
| DELETE | `/v1/personas/{id}` | subject | | `204` |
| PUT | `/v1/personas/{id}/fields/{key}` | subject | `{value, sensitivity}` | `Persona` |
| DELETE | `/v1/personas/{id}/fields/{key}` | subject | | `Persona` |
| GET | `/v1/requesters` | subject | `?mine=true` for own only | `[Requester]` (no secrets) |
| POST | `/v1/requesters` | subject | `{name}` | `201 Requester + client_secret` (once) |
| POST | `/v1/requesters/{id}/secret` | subject (owner) | | `{client_secret}` |
| GET | `/v1/rules` | subject | | `[Rule]` |
| POST | `/v1/rules` | subject | `{requester_id?, purpose?, persona_id, max_sensitivity, allow_keys?, priority?}` | `201 Rule` |
| DELETE | `/v1/rules/{id}` | subject | | `204` |
| GET | `/v1/audit` | subject | `?limit=50&before=<seq>` | `{decisions:[AuditEvent], next_before}` |
| GET | `/v1/audit/verify` | subject | | `{ok, length, broken_at_seq?, head_hash}` |

`Session`:
```json
{ "access_token": "…", "refresh_token": "…",
  "access_expires_at": "…", "refresh_expires_at": "…",
  "subject": { "id": "…", "handle": "alice", "display_name": "Alice" },
  "roles": ["subject"], "redirect_to": "/app" }
```

`Rule.allow_keys`: omit/`null` = ceiling only; `[]` = release nothing.

`AuditEvent`: `{seq, event_id, requester_id, purpose, allowed, reason,
persona_id?, persona_label?, disclosed_keys, withheld_keys, decided_at,
recorded_at, hash, prev_hash}` (hashes hex).

## Machine listener — requester traffic (`api.afixo.io`)

CORS allows `https://afixo.io` (the dashboard's Explorer acts as a requester
from the browser). Responses that carry tokens or personal data are
`Cache-Control: no-store`.

| Method | Path | Auth | Body / query | Response |
|---|---|---|---|---|
| GET | `/v1/health` | — | | |
| GET | `/v1/purposes` | — | | `[{name, description}]` |
| POST | `/oauth/token` | client credentials: HTTP Basic **or** form `client_id`/`client_secret` | form `grant_type=client_credentials` | `{"access_token","token_type":"Bearer","expires_in"}` |
| GET | `/v1/disclose/{handle}` | `Bearer <requester token>` | `?purpose=<name>` (required) | see below |

Disclose:

```
200 { "decision": "allow", "decision_id": "…", "persona": "legal",
      "fields": { "full_name": "…", "email": "…" }, "withheld": ["dob", "postal_address"] }

403 { "decision": "deny", "decision_id": "…", "reason": "no_matching_rule" }
```

The `403` is identical for "no rule", "unknown handle" and "persona gone".
Unknown purpose → `400 invalid_purpose`. No/invalid token → `401 invalid_token`.
A subject token on this listener → `403 wrong_principal`; a requester token on
the console listener likewise. Any console path on `api.afixo.io` is a `404`
from the api Worker before it ever reaches the cluster.

## Status codes from gRPC

| gRPC | HTTP |
|---|---|
| INVALID_ARGUMENT | 400 `invalid_request` |
| UNAUTHENTICATED | 401 `invalid_token` |
| PERMISSION_DENIED | 403 `forbidden` |
| NOT_FOUND | 404 `not_found` |
| ALREADY_EXISTS / FAILED_PRECONDITION | 409 |
| RESOURCE_EXHAUSTED | 429 |
| UNIMPLEMENTED | 501 `not_implemented` (skeleton services) |
| UNAVAILABLE / DEADLINE_EXCEEDED | 503 `upstream_unavailable` (also: audit down ⇒ disclose fails closed) |
| anything else | 500 `internal` (details logged, never returned) |
