# afixo-services

Rust workspace for the Afixo backend: six services behind one in-cluster
gateway, deployed to DigitalOcean Kubernetes (`k8s-afixo-io-fra1`; namespaces
`afixo` = production, `afixo-staging` = staging) and reachable **only** through
a Cloudflare Tunnel that only the `afixo-api` Worker may open. The cluster has
no public address, no ingress controller and no load balancer, and no public
hostname resolves to the tunnel.

- Product spec (what the system must do): `FINAL_REPORT.md` in the workspace
  root — `../FINAL_REPORT.md` when this repo is checked out beside `afixo-web`
  and `afixo-api`.
- Design docs in this repo: `docs/architecture.md` (system + hostnames),
  `docs/domain.md` (model + decision algorithm), `docs/api.md` (REST contract),
  `docs/audit.md` (synchronous audit + hash chain), `docs/deploy.md`
  (DigitalOcean + Cloudflare, both environments).

## The shape

```
browser ─► afixo.io ──┐
client  ─► api.afixo.io ┴► afixo-web ─binding─► afixo-api ─► origin(-api).afixo.io (Access) ─► tunnel ─► cloudflared ─► gateway ─gRPC─► auth · identity · policy · disclosure ─► audit
```

| Service | Owns | Talks to |
|---|---|---|
| `gateway` | nothing — REST ↔ gRPC, two listeners (`:8080` console, `:8081` machine) | every service |
| `auth` | GitHub identities, sessions (rotating refresh), requesters, requester tokens | identity |
| `identity` | subjects, personas, persona fields | — |
| `policy` | purpose vocabulary, disclosure rules | identity, auth (validation at write time) |
| `disclosure` | nothing — the hot path; runs `afixo-engine`, then `audit.Record`, then answers | identity, policy, audit |
| `audit` | hash-chained decision log (`Record` idempotent on `event_id`, reads, verify) | — |

Shared crates: `crates/proto` (generated contract), `crates/common` (boot
plumbing), `crates/engine` (the pure decision, property-tested).

## Hard rules

1. **Deny by default.** No rule → deny. Missing/foreign persona → deny. Unknown
   subject → deny, and the gateway answers a uniform `403` so handles cannot be
   enumerated. Rules only grant; there is no deny rule.
2. **The service that owns the data re-checks ownership.** Every mutating RPC
   carries the acting `subject_id` and the owning service puts it in the SQL
   predicate. The gateway authenticating the caller is not a substitute.
3. **Tokens and secrets are stored hashed (SHA-256), compared in constant time,
   and never logged.** Refresh tokens rotate on every use; reuse revokes the
   whole session family. Do not log cookie values, tokens, or persona field values.
4. **No unlogged disclosure.** `disclosure` awaits `audit.Record` before
   answering; if `audit` is down the request fails closed (`503`). `Record` is
   idempotent on `event_id`. Never make the audit write asynchronous or
   best-effort.
5. **Cross-service references are plain uuids**, validated by RPC at write
   time. Never add a foreign key across databases; never let one service read
   another service's database.
6. **The proto files are the contract.** Change `proto/afixo/v1/*.proto` first;
   never edit generated code.
7. **sqlx 0.9:** SQL must be a `&'static str` literal — no `format!`. Queries are
   runtime-checked (`query_as` + `FromRow`); migrations live in
   `services/<svc>/migrations`, are embedded with `sqlx::migrate!` and run at boot.
8. **Nothing in `deploy/` may expose the cluster**: no Service of type
   LoadBalancer/NodePort, no Ingress, no hostPort. Public hostnames are
   Workers; only `origin*` hostnames route to a tunnel, and each sits behind
   an Access Service Auth policy.
9. **Ask before `git commit`; push only when the user asks.** Default branch is `master`; commits are authored as the user (`valfz`).

## Commands

```sh
make check          # cargo check --workspace --all-targets
make lint           # cargo fmt --check + clippy -D warnings   (CI gate)
make test           # all tests; the #[sqlx::test] suites need `make infra-up` + DATABASE_URL (.env)
make infra-up       # postgres via docker compose
make migrate        # apply every service's migrations locally
make run-policy     # one service (reads .env; maps POLICY_DATABASE_URL → DATABASE_URL)
make dev            # all six services locally (ports in .env.example)
make docker-gateway # build one image; make docker-all for all
make k8s-diff ENV=staging   # kubectl diff -k deploy/k8s/overlays/staging (ENV=prod for production)
```

Needs: Rust 1.96 (rust-toolchain.toml), `protoc`, Docker, `sqlx-cli`
(`cargo install sqlx-cli --no-default-features --features postgres`).

## Layout

```
proto/afixo/v1/        common · auth · identity · policy · disclosure · audit
crates/{proto,common,engine}
services/{gateway,auth,identity,policy,disclosure,audit}   each: CLAUDE.md, src/, migrations/ (if it owns a db)
deploy/k8s/base        one file per workload + network policies + kustomization (environment-agnostic)
deploy/k8s/overlays/{prod,staging}   namespace, hostnames, pull secret, image tags
deploy/scripts         create-secrets.sh · cloudflare-tunnel-setup.sh <env> · verify-edge.sh <env>
deploy/local           init-databases.sql (compose)
docs/                  architecture · domain · api · audit · deploy
Dockerfile             one multi-stage build for every service (--build-arg SERVICE=…)
compose.yml            local infra; --profile full runs everything in containers
```

## Conventions

- Edition 2024, clippy `pedantic` at warn level, `-D warnings` in CI.
- Config is environment-only (`afixo_common::config`); a missing required
  variable fails boot. Secrets arrive through Kubernetes Secrets, never files.
- Errors: inside services, `tonic::Status` with the semantic code
  (`not_found`, `invalid_argument`, `unauthenticated`, `already_exists`…);
  `afixo_common::grpc::internal` for anything unexpected (logs the cause,
  returns an opaque INTERNAL). The gateway maps codes to HTTP in `error.rs`.
- Ids are UUID v7 (`afixo_common::ids::new_id`). Timestamps `timestamptz` ↔
  `time::OffsetDateTime` ↔ `google.protobuf.Timestamp`.
- Tracing: `tracing` everywhere, JSON in the cluster (`LOG_FORMAT=json`).
  Log ids and outcomes, never payloads.
- Tests: pure logic gets unit + `proptest` (see `crates/engine`). Data
  services use `#[sqlx::test(migrations = "./migrations")]`: one throw-away
  database per test on the server named by `DATABASE_URL` (CI provides it), so
  tests never share state and never truncate. Services that only orchestrate
  (`disclosure`, `auth`'s GitHub/identity hops) test against in-process tonic /
  axum stand-ins on ephemeral ports.

## Status (2026-08-22)

All six services are implemented: `gateway` (REST, two listeners), `auth`
(GitHub login, rotating sessions, requesters, client tokens, introspection),
`identity`, `policy`, `disclosure` (orchestration + synchronous audit) and
`audit` (hash-chained log, `Record`/`ListDecisions`/`VerifyChain`), plus
`afixo-engine`. Tests: unit + property tests (no infra), and `#[sqlx::test]`
integration tests per data service — each test gets a throw-away database on
the server named by `DATABASE_URL` (`make infra-up`, then `make test`; CI
provides it). `disclosure` and `auth` test against in-process tonic/axum
stand-ins. Not yet done: gateway integration tests, the Access-JWT check at
the gateway, Criterion/load measurements.

## Git

`origin` = `git@github.com:afixo/services.git`, branch `master`. Ask before
committing; push only when asked. Commit messages: imperative, scoped by
service (`auth: rotate refresh tokens`).
