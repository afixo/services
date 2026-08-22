# afixo-services

Backend of **Afixo** — a selective-disclosure identity platform. People keep
several context-specific *personas*; machine clients ask for a person under a
declared *purpose* and receive exactly the persona and fields the person's
policy allows. Deny by default, every decision in a hash-chained audit log.

```
GET https://api.afixo.io/v1/disclose/alice?purpose=shipping
Authorization: Bearer <requester token>

200 { "decision": "allow", "persona": "legal",
      "fields": { "full_name": "Alice Example", "postal_address": "…" },
      "withheld": ["dob", "email"], "decision_id": "…" }
```

Rust · tonic/gRPC · axum · PostgreSQL · Kubernetes (DigitalOcean) · Cloudflare Workers + Tunnel (separate repos: `afixo-web`, `afixo-api`). Nothing public resolves to the cluster.

Six services — `gateway`, `auth`, `identity`, `policy`, `disclosure`, `audit` —
each owning its data, sharing a generated contract (`crates/proto`) and one
pure, property-tested decision engine (`crates/engine`).

Start with [`CLAUDE.md`](CLAUDE.md) (operational guide) and
[`docs/architecture.md`](docs/architecture.md).

## Quick start

```sh
cp .env.example .env
make infra-up && make migrate
make dev                      # six services on localhost
curl -s localhost:8080/v1/purposes | jq
```
