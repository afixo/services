# afixo-common

Boot-time plumbing every service shares. Nothing here knows about subjects,
personas or rules — if a function needs a domain type, it does not belong here.

| Module | Provides |
|---|---|
| `config` | `required`, `or`, `optional`, `parse_or`, `list`, `grpc_addr` — env-only configuration; missing required ⇒ boot failure |
| `telemetry` | `init(service)` — `RUST_LOG` filter, `LOG_FORMAT=json` for the cluster |
| `db` | `connect()` — `PgPool` from `DATABASE_URL`, `DB_MAX_CONNECTIONS` |
| `grpc` | `shutdown_signal`, `health::<S>()`, `channel("X_URL")` (lazy), `internal(ctx, err)`, `db_status(ctx, err)` |
| `ids` | `new_id()` (uuid v7), `parse`, `parse_opt`, `validate_handle` |
| `tokens` | `random_token` (256-bit, base64url), `random_client_id` (`afx_…`), `sha256`, `digest_eq` (constant time) |
| `time` | `now`, `to_proto`, `to_proto_opt`, `from_proto` |

## Rules

- `grpc::internal` is the only way to turn an unexpected error into a
  `Status`: it logs the cause and returns an opaque message. Do not
  `Status::internal(format!("{e}"))` in services.
- `db_status` keeps `RowNotFound` → `NOT_FOUND` and unique violations →
  `ALREADY_EXISTS`; use it on every sqlx error.
- Randomness comes from `getrandom` (OS CSPRNG). Never `rand::thread_rng` for
  tokens.
- Adding a dependency here adds it to every service; prefer putting it in the
  one service that needs it (e.g. `reqwest` belongs in `auth`).
