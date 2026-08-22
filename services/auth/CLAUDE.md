# auth

Credentials and tokens for both caller types. Status: **implemented** — all
ten RPCs, hourly cleanup, integration tests against Postgres with stand-ins
for GitHub and identity.

Owns (db `afixo_auth`): `github_identities`, `oauth_states`, `sessions`,
`requesters`, `requester_tokens`. Calls `identity.ResolveSubject` /
`EnsureSubject` / `GetSubject` during login and refresh.

## Invariants

1. Nothing secret is stored in clear. Access/refresh/requester tokens and
   client secrets are 256-bit random (`afixo_common::tokens::random_token`),
   returned once, stored as SHA-256 (`tokens::sha256`), compared with
   `tokens::digest_eq` (constant time; an unknown `client_id` is compared
   against a dummy digest so the answer and the work are the same).
2. **Refresh rotation with reuse detection.** A refresh marks the consumed
   row `rotated_at` and inserts its successor in the same `family_id`, in one
   transaction. Presenting a refresh token whose row is already rotated is a
   replay ⇒ `revoke_family` (every row, including the successor), answer
   `UNAUTHENTICATED("refresh_reuse")`. This is why the edge must never
   refresh on its own.
3. `IntrospectToken` is one lookup by hash: `sessions.access_hash` (not
   revoked, not expired) then `requester_tokens.token_hash` (not expired).
   Inactive tokens return `active=false`, never an error. A refresh token is
   never a bearer.
4. OAuth `state` is single-use with a 10-minute TTL: `delete … returning` in
   the same statement that reads it; a failed code exchange still consumes it.
5. Only the owner rotates a requester's secret (`where id = $1 and
   owner_subject_id = $2`); a foreign id is `NOT_FOUND`. Rotation deletes the
   requester's live tokens. Listing without an owner is the public directory
   (never hashes).
6. **A new GitHub account never inherits an existing handle.** The
   `github_id → subject_id` link is authoritative: a renamed GitHub login keeps
   its subject; a *different* account whose login equals an existing handle
   gets `<login>-<4 hex>`.
7. Lifetimes (`settings.rs`): access 15 min, refresh 7 d, requester token 1 h,
   state 10 min. Expiry is a column and part of every lookup predicate; the
   hourly `cleanup` task only deletes rows that can never match again.
8. Log `subject_id`/`requester_id`/`family_id` and outcomes. Never tokens,
   never the GitHub access token, never the client secret.

## How it is built

```
src/main.rs       boot, migrate, GitHub config from env, cleanup task, serve (+ health)
src/settings.rs   GithubConfig (GITHUB_CLIENT_ID/SECRET/REDIRECT_URI; GITHUB_OAUTH_BASE/API_BASE for tests) + TTLs
src/github.rs     authorize URL, code exchange (form, Accept: json), GET /user — reqwest (rustls)
src/repo.rs       literal-SQL queries per table; transactions for rotate_session and rotate_secret
src/server.rs     AuthService impl; subject_for()/free_handle() implement invariant 6
src/cleanup.rs    hourly delete_expired
tests/auth.rs     #[sqlx::test] end-to-end flows with an axum GitHub stub and a tonic identity stub
migrations/       0001: the five tables
```

Env beyond the common set: `IDENTITY_URL`, `GITHUB_CLIENT_ID`,
`GITHUB_CLIENT_SECRET`, `GITHUB_REDIRECT_URI` (`https://afixo.io/api/v1/auth/github/callback`;
staging uses `staging.afixo.io`).

## Tests

```sh
docker compose up -d postgres
DATABASE_URL=postgres://afixo:afixo@localhost:5432/afixo cargo test -p afixo-auth
```

Covered: login creates subject + session (lower-cased handle, roles,
redirect_to); state single-use and bad codes; returning user after a GitHub
rename; new account vs. existing handle; refresh rotation and reuse revoking
the family; expired refresh / expired access; logout idempotent; client
credentials → token → introspection, wrong/unknown credentials; rotation is
owner-only, kills old secret and tokens, keeps `client_id`; directory vs.
owner-scoped listing; cleanup deletes only expired rows.

Run: `make run-auth`.
