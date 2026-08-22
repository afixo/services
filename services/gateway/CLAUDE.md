# gateway

The REST edge inside the cluster. Owns no data. Translates HTTP ↔ gRPC, maps
errors, and authenticates every bearer token by asking `auth.IntrospectToken`.
Status: **all routes wired**; end-to-end works for whatever the upstream
services implement (today: `/v1/health`, `/v1/purposes`, rules CRUD once
identity/auth validation RPCs exist).

## Two listeners, one process

| Listener | Env | Reached via | Surface |
|---|---|---|---|
| console | `CONSOLE_ADDR` (`0.0.0.0:8080`) | `afixo.io/api/*` → afixo-api (console mode) → `origin.afixo.io` (Access) → tunnel | everything a subject does (`routes/console.rs`) |
| machine | `MACHINE_ADDR` (`0.0.0.0:8081`) | `api.afixo.io` → afixo-api (machine mode) → `origin-api.afixo.io` (Access) → tunnel | `/v1/health`, `/v1/purposes`, `POST /oauth/token`, `GET /v1/disclose/{handle}` (`routes/machine.rs`) |

cloudflared routes each origin hostname to its port. **Never add a subject
route to the machine router** and never merge the two — the split is the
product's security boundary for the machine hostname (the api Worker also
refuses console paths on `api.afixo.io`, but this router must not rely on it).

## Rules

- Auth is an extractor: `auth::Subject` (console) / `auth::Requester`
  (machine). Both call `IntrospectToken`; a token of the wrong kind on a
  listener is `403 wrong_principal`. Handlers never read `Authorization` themselves.
- Every handler passes the authenticated `subject_id`/`requester_id`
  explicitly in the request message. No gRPC metadata magic.
- Deny on disclose is a **uniform 403** (`routes/machine.rs::disclose`). Do not
  leak `unknown_subject` vs `no_matching_rule` to requesters.
- Error mapping lives only in `error.rs` (`From<tonic::Status>`); handler-local
  `map_err` is for semantic overrides (e.g. `invalid_client`).
- Token and disclosure responses are `Cache-Control: no-store`.
- CORS exists only on the machine listener (`CORS_ALLOWED_ORIGINS`).
- Request id: `x-request-id` in/out (`tower-http`); one `TraceLayer` span per request.

## Env

`CONSOLE_ADDR`, `MACHINE_ADDR`, `AUTH_URL`, `IDENTITY_URL`, `POLICY_URL`,
`DISCLOSURE_URL`, `AUDIT_URL` (lazy channels — boot order does not matter),
`CORS_ALLOWED_ORIGINS` (default `https://afixo.io`), `RUST_LOG`, `LOG_FORMAT`.

## Layout

```
src/main.rs          two TcpListeners, graceful shutdown on SIGTERM
src/state.rs         AppState: one client per upstream
src/auth.rs          Subject / Requester extractors, ClientCredentials (Basic or form)
src/error.rs         ApiError + gRPC→HTTP mapping
src/dto.rs           JSON shapes (docs/api.md is derived from these)
src/routes/console.rs · machine.rs · mod.rs (health, common layers)
```

## Next

- Validate `Cf-Access-Jwt-Assertion` on the console listener (config
  `ACCESS_TEAM_DOMAIN`, `ACCESS_AUD`; JWKS cache) — defence in depth.
- Integration tests: spin the router with mocked tonic servers
  (`tonic::transport::Server` on an ephemeral port) and assert the status
  mapping, the uniform 403, and the wrong-principal cases.
- Per-route timeouts (`tower_http::timeout`) once latency is measured.

Run: `make run-gateway` (needs the other services or expect 503s).
Check: `curl -s localhost:8080/v1/health`, `curl -s localhost:8081/v1/purposes`.
