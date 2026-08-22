# Deployment — DigitalOcean + Cloudflare

State on 2026-08-22: DOKS cluster **`k8s-afixo-io-fra1`** (id
`17bbaf36-1e39-4d2e-b436-7f0179d6a269`, Kubernetes 1.36, one `s-1vcpu-2gb`
node: 920m CPU / ~1.5 GiB allocatable, of which system pods request 512m /
582Mi). No container registry and no managed database existed yet.

Two environments on that cluster: **production** (namespace `afixo`) and
**staging** (namespace `afixo-staging`). Same images; different namespace,
tunnel, databases, secrets and Worker hostnames.

## What the cluster needs from DigitalOcean

| Resource | How | Used by |
|---|---|---|
| DOKS cluster | exists | both environments |
| PostgreSQL | **in the cluster**, not managed: `deploy/k8s/base/postgres.yaml` runs `postgres:17-alpine` as a StatefulSet per namespace on a 10 Gi `do-block-storage-retain` volume (~$1/month; the PV survives deleting the StatefulSet or namespace). Databases `afixo_auth/identity/policy/audit` are created on first boot from `postgres-init`. No backups beyond the volume — take `pg_dump`s if the data matters. | `POSTGRES_PASSWORD` in `afixo-secrets`; the `*_DATABASE_URL`s are derived by `create-secrets.sh` |
| CI deploy identity, **per namespace** | `deploy/scripts/make-ci-kubeconfig.sh <staging\|production>` — a `github-deployer` ServiceAccount confined to the namespace (no Secrets, no delete, no other namespaces); uploads its kubeconfig as the GitHub environment secret `KUBE_CONFIG` | `deploy.yml` (no DigitalOcean API token needed) |

There is **no DigitalOcean container registry**. Images live in GitHub's
registry as private packages `ghcr.io/afixo/services/<service>`, pushed by
`deploy.yml` with its own `GITHUB_TOKEN` (`permissions: packages: write`) —
nothing to create on the push side. Pulling needs a credential in the cluster:

| Resource | How | Used by |
|---|---|---|
| GitHub PAT for pulls | classic PAT, scope **`read:packages` only**, no expiry or a long one (rotate = recreate both Secrets below) | the pull secret |
| Pull secret `registry-afixo`, **per namespace** | `deploy/scripts/create-pull-secret.sh <PAT> [production\|staging]` (creates the namespace if needed; re-run to rotate) | every Deployment (`imagePullSecrets: registry-afixo`, patched in the overlays) |

After the first push, each package appears under the org's *Packages* tab as
private; the `org.opencontainers.image.source` label in the `Dockerfile` links
it to this repo so the workflow keeps write access on later pushes.

No load balancer, no inbound firewall rule — keep the default
DOKS firewall object; it stays empty.

## What Cloudflare needs

1. **Tunnels** — `deploy/scripts/cloudflare-tunnel-setup.sh production` and
   `… staging`: creates `afixo-fra1` / `afixo-fra1-staging`, routes only the
   `origin*` hostnames to them, writes each namespace's
   `cloudflared-credentials` Secret. Never route a public hostname to a tunnel.
2. **Access** (dashboard click-path in `afixo-api/docs/access.md`) — one
   service token per environment; a self-hosted application with a single
   **Service Auth** policy on *each* of `origin.afixo.io`, `origin-api.afixo.io`
   (and the two staging ones); an **Allow-by-email** application on
   `staging.afixo.io`; nothing on `afixo.io`, `api.afixo.io`,
   `api-staging.afixo.io`; never a wildcard.
3. **Rate limiting** — WAF rate-limiting rules on `api.afixo.io` (e.g.
   `/oauth/token` 10/min per IP, `/v1/disclose/*` 120/min per IP).
4. **Workers** — `afixo-api` first (secrets via its `scripts/push-secrets.sh`),
   then `afixo-web` (custom domains `afixo.io`, `www.afixo.io`, `api.afixo.io`;
   staging: `staging.afixo.io`, `api-staging.afixo.io`), `afixo-docs`
   (`docs.afixo.io`, repo `documents`) and `afixo-mcp` (`mcp.afixo.io`, binds
   to `afixo-api`). Delete any existing A/CNAME for those names first — a
   custom domain refuses to attach over a record.
5. **GitHub OAuth apps** — one per environment; callbacks
   `https://afixo.io/api/v1/auth/github/callback` and
   `https://staging.afixo.io/api/v1/auth/github/callback`; ids/secrets go into
   the respective `afixo-secrets`.

## First deploy, in order (per environment)

```sh
ns=afixo            # or afixo-staging
kubectl create ns $ns
deploy/scripts/create-pull-secret.sh <PAT read:packages> production                         # → registry-afixo
deploy/scripts/make-ci-kubeconfig.sh production                                            # or staging → KUBE_CONFIG
NAMESPACE=$ns deploy/scripts/create-secrets.sh deploy/k8s/overlays/prod/secrets/.env.prod   # gitignored file
deploy/scripts/cloudflare-tunnel-setup.sh production                                       # or staging
# images: let deploy.yml build them (push to master → staging; dispatch → production),
# or locally: make docker-all && docker push …
kubectl apply -k deploy/k8s/overlays/prod                                                  # or staging
make k8s-status ENV=prod
deploy/scripts/verify-edge.sh production
```

GitHub secrets for `afixo/services`: only `KUBE_CONFIG`, as an **environment**
secret on each of `staging` and `production` (`make-ci-kubeconfig.sh` creates
the environment and uploads it). Image pushes use the workflow's own
`GITHUB_TOKEN`. Optional repository variable: `IMAGE_REGISTRY` (default
`ghcr.io/afixo/services`; changing it also means changing the image names in
`deploy/k8s`). Put a required reviewer on the `production` environment if you
want a manual gate.

## CI/CD behaviour

- `ci.yml` (PRs + master): fmt, clippy `-D warnings`, tests (Postgres
  available), `cargo deny`, both overlays rendered, a no-push Docker build of
  `gateway`.
- `deploy.yml`: push to `master` → **staging**; `workflow_dispatch` →
  staging or **production**. Builds only the services whose sources changed
  — **any change under `crates/`, `proto/`, `Cargo.*` or `Dockerfile`
  rebuilds all six** — pushes `:<sha>` and `:latest`, pins the tags in the
  chosen overlay, `kubectl apply -k`, waits for rollouts in that namespace.
- Migrations run at service boot (`sqlx::migrate!`), so a deploy that ships a
  migration applies it on first start. Write migrations to be
  backwards-compatible with the previous binary (add, don't rename) because
  the old pod is still serving during the rollout.

## Sizing notes for the 1-vCPU node

Requests: 25m/32Mi per Rust service and cloudflared, 100m/160Mi for Postgres — ~275m / ~385Mi per
environment, ~550m / ~770Mi for both, leaving room for surge pods during
rolling updates. Memory limits only (128Mi); no CPU limits, to avoid
throttling a single-core node. Raise the pool to two nodes before adding
replicas — a second replica on the same node buys nothing.

## Follow-ups (not done)

- Validate `Cf-Access-Jwt-Assertion` at the gateway (JWKS from
  `https://<team>.cloudflareaccess.com/cdn-cgi/access/certs`) as defence in
  depth behind Access.
- Backups: the in-cluster Postgres has none beyond its block volume; the audit database is the
  record — back it up and test a restore.
- Observability beyond logs: OpenTelemetry traces (`tracing-opentelemetry`)
  and Prometheus metrics are not wired.
