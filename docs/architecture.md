# Architecture

Afixo is the selective-disclosure identity API from `FINAL_REPORT.md`
(one TypeScript Worker + D1/KV + Vue) rebuilt as a distributed system. The
thesis is unchanged — given *(subject, requester, purpose)* select a rule,
resolve the persona, apply ceiling + allow-list, deny by default, audit the
decision — and is now one pure crate (`afixo-engine`) surrounded by services
that own data, a gateway that owns HTTP, and an edge that owns the session.

## The one rule of the topology

**Nothing an end user or an API client talks to resolves to the cluster.**
Every public hostname is a Cloudflare Worker. The Workers reach the cluster
through a Cloudflare Tunnel whose hostnames sit behind Cloudflare Access
(service token) and are called by exactly one Worker. The cluster has no
public IP, no ingress controller, no load balancer and no inbound firewall rule.

```
                    ┌──────────────────────── Cloudflare ────────────────────────┐
 browser ── afixo.io ─────► afixo-web (Astro Worker)                            │
 client ─── api.afixo.io ─► afixo-web: src/fetch.ts branches on hostname        │
                    │            └─ service binding ─► afixo-api (no public route)│
                    │                    console mode: cookie → bearer, CSRF     │
                    │                    machine mode: bearer passed through     │
                    │                                 │                          │
                    │          origin.afixo.io ◄───────┤   Access: Service Auth   │
                    │          origin-api.afixo.io ◄───┘   Access: Service Auth   │
                    └───────────────────┬────────────────┬────────────────────────┘
                                        │ tunnel         │
            ┌── DOKS fra1, namespace afixo ──────────────┼────────────────────────────┐
            │  cloudflared ─► gateway :8080 (console) / :8081 (machine)               │
            │                    │ gRPC                                               │
            │     auth ◄─────────┼────────► identity      policy ◄── disclosure ──► audit
            │      │             │              ▲            ▲         │               │
            │      └── identity ─┘              └────────────┴─────────┘               │
            └──────────────────────────────────────────────────────────────────────────┘
                 DigitalOcean managed PostgreSQL: afixo_auth · afixo_identity · afixo_policy · afixo_audit
```

| Component | Repo | Role |
|---|---|---|
| `afixo-web` | afixo-web | Public Worker: custom domains `afixo.io`, `www.afixo.io`, **`api.afixo.io`**. Static site + dashboard. `src/fetch.ts` forwards any request on a machine host, and `/api/*` on the site host, to `afixo-api` over a service binding with the original URL. Holds no secrets. |
| `afixo-api` | afixo-api | Session boundary, no public route. **Console mode** (site host): sealed-cookie ⇄ bearer, Origin + CSRF, Access headers, → `origin.afixo.io`. **Machine mode** (`api.afixo.io`): only `/oauth/token`, `/v1/disclose/*`, `/v1/purposes`, `/v1/health`; the client's own `Authorization` passed through, cookies stripped, Access headers added, → `origin-api.afixo.io`. |
| `gateway` | here | REST ↔ gRPC. Two listeners so the subject surface and the machine surface are different sockets, routed by cloudflared per origin hostname. |
| `auth` | here | Subjects: GitHub OAuth → opaque access (15 min) + rotating refresh (7 d). Requesters: registry + client-credentials → opaque tokens (1 h). `IntrospectToken` is how the gateway learns who is calling. |
| `identity` | here | Subjects, personas, fields with sensitivity tiers. |
| `policy` | here | Purpose vocabulary and rules. Stores and selects candidates; never ranks. |
| `disclosure` | here | The product hot path. Resolves subject → candidates → winner → persona → engine → `audit.Record` → answer. Owns no data. |
| `audit` | here | Append-only, hash-chained log of every decision; `Record` (idempotent), paginated reads, chain verification. |

## Hostnames

| Hostname | Is | Auth at that hop |
|---|---|---|
| `afixo.io`, `www.afixo.io` | Worker `afixo-web` custom domains | — (dashboard gated by the session cookie) |
| `api.afixo.io` | Worker `afixo-web` custom domain (machine host) | bearer / client credentials, validated by the gateway |
| `origin.afixo.io` | Tunnel → `gateway:8080` | Cloudflare Access, Service Auth (afixo-api's token) |
| `origin-api.afixo.io` | Tunnel → `gateway:8081` | Cloudflare Access, Service Auth (afixo-api's token) |
| `docs.afixo.io` | Worker `afixo-docs` (static docs site, repo `documents`) | — |
| `mcp.afixo.io` | Worker `afixo-mcp` — MCP server at `/mcp`; calls the machine API through `afixo-api` with the client's own bearer | bearer (requester token) passed through |
| `staging.afixo.io` | Worker `afixo-web-staging` custom domain | Cloudflare Access, Allow by email (+ the session cookie) |
| `api-staging.afixo.io` | Worker `afixo-web-staging` custom domain (machine host) | bearer / client credentials |
| `origin-staging.afixo.io`, `origin-api-staging.afixo.io` | Tunnel (staging) → staging gateway | Cloudflare Access, Service Auth (staging token) |

No hostname is a CNAME to the tunnel except the four `origin*` ones, and
those reject anything without the Worker's service token.

## Request paths

**Dashboard (subject).** `browser → afixo-web → [binding] → afixo-api
(console mode) → https://origin.afixo.io/v1/… → tunnel → gateway:8080 →
gRPC`. The Worker translates cookie → bearer; the gateway introspects the
bearer with `auth`; the owning service re-checks `subject_id` in SQL.

**Product (requester).** `client → https://api.afixo.io → afixo-web
(src/fetch.ts) → [binding] → afixo-api (machine mode) →
https://origin-api.afixo.io → tunnel → gateway:8081`. `POST /oauth/token`
(client credentials) then `GET /v1/disclose/:handle?purpose=`. The gateway
introspects the bearer, `disclosure` orchestrates, `audit.Record` is awaited
before the answer (`docs/audit.md`).

The dashboard's Explorer acts as a requester from the browser: it calls
`https://api.afixo.io` cross-origin, so the gateway's machine listener answers
CORS for `https://afixo.io` (relayed unchanged by the Workers).

## Trust boundaries

| Boundary | Enforced by |
|---|---|
| Internet → cluster | Nothing inbound. cloudflared dials out; every tunnel hostname is behind Access (service token only). The public hostnames are Workers. |
| Worker → origin | Access service token (`CF-Access-Client-Id/Secret`), one Access application per origin hostname, never a wildcard. |
| gateway → services | Private network + NetworkPolicy (only `afixo.io/tier: backend` pods reach `:50051`). Services trust the gateway's authenticated `subject_id`/`requester_id` **and** re-check ownership. |
| services → data | One database per service; no shared schema; no cross-db foreign keys. |

Not done (and stated so): no mTLS/service mesh between pods; the Access JWT
(`Cf-Access-Jwt-Assertion`) is not yet validated at the gateway as defence in
depth — both listed in `docs/deploy.md`.

## Why these choices

- **Six services, not fifteen.** Boundaries follow *ownership and workload*:
  credentials vs. profile data vs. policy vs. a read-heavy orchestrator vs. an
  append-only log. `persona-service`/`rule-service`/… would be distributed CRUD.
- **The engine is a crate, not a service.** Ranking and filtering are pure and
  property-tested. Keeping them in one function preserves the report's
  argument for ranking in code rather than SQL.
- **Synchronous audit, no broker.** The report's guarantee ("no unlogged
  disclosure") is kept by awaiting `audit.Record`; audit down ⇒ disclose fails
  closed. A JetStream design was considered and dropped: same guarantee, more
  machinery, one more pod on a one-core node (`docs/audit.md`).
- **No Redis.** Tokens live in Postgres with expiry columns; rate limiting is
  a Cloudflare rule; there is no cache.
- **Opaque tokens.** The only claim anyone needs is "which principal";
  revocation is a row; no signing keys. Introspection is one indexed lookup by hash.
- **Two gateway listeners, two origin hostnames.** The subject surface and
  the machine surface are different sockets; cloudflared routes by hostname;
  nothing parses `Host`.
- **Machine traffic through the Workers too.** `api.afixo.io` could have been
  a tunnel hostname; it is a Worker custom domain instead so that *no* public
  name ever resolves to the tunnel, and so the Access service token is the
  only credential that can open it.
- **Sealed cookie at the edge, no refresh at the edge.** Worker invocations
  cannot coordinate; the browser single-flights a refresh.

## Environments

| | production | staging |
|---|---|---|
| Workers | `afixo-web`, `afixo-api` | `afixo-web-staging` (`staging.afixo.io`, `api-staging.afixo.io`, behind Access email policy), `afixo-api-staging` |
| Namespace | `afixo` | `afixo-staging` |
| Tunnel | `afixo-fra1` → `origin.` / `origin-api.` | `afixo-fra1-staging` → `origin-staging.` / `origin-api-staging.` |
| Databases | `afixo_*` | `afixo_*_staging` on the same managed cluster |
| Deployed by | `workflow_dispatch` → production (in every repo) | push to `master` (in every repo) |

Both namespaces share the single `s-1vcpu-2gb` node for now (requests are
sized to fit: ~175m CPU / ~225Mi per environment); add a node before adding
replicas. Local development uses docker compose for Postgres and `cargo run`
for services.
