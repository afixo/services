# auth

Credentials and tokens for both caller types. Status: **skeleton** — schema and
contract final, every RPC answers `UNIMPLEMENTED`.

Owns (db `afixo_auth`): `github_identities`, `oauth_states`, `sessions`,
`requesters`, `requester_tokens`. Calls `identity.EnsureSubject` on first login.

## Invariants

1. Nothing secret is stored in clear. Access/refresh/requester tokens and
   client secrets are 256-bit random (`afixo_common::tokens::random_token`),
   returned once, stored as SHA-256 (`tokens::sha256`), compared with
   `tokens::digest_eq` (constant time).
2. **Refresh rotation with reuse detection.** A refresh creates a new `sessions`
   row in the same `family_id` and sets `rotated_at` on the old one. Presenting
   a refresh token whose row is already rotated = replay ⇒ revoke every row in
   the family, answer `UNAUTHENTICATED`. This is why the edge must never
   refresh on its own (two racing refreshes would look like theft).
3. `IntrospectToken` is one lookup by hash across `sessions.access_hash` (not
   revoked, not expired) then `requester_tokens.token_hash` (not expired).
   Inactive tokens return `active=false`, never an error.
4. OAuth `state` is single-use with a 10-minute TTL; the callback deletes it
   in the same statement that reads it (`delete … returning`).
5. Only the owner rotates a requester's secret
   (`where id = $1 and owner_subject_id = $2`). Listing without owner returns
   the public directory (id, name, owner) — never hashes.
6. Lifetimes: access 15 min, refresh 7 d, requester token 1 h, state 10 min.
   Expiry is a column and part of every lookup predicate; a periodic task
   deletes expired rows (`tokio::time::interval`, hourly) — it is hygiene,
   not security.
7. Log `subject_id`/`requester_id`/`family_id` and outcomes. Never tokens.

## Implementation plan

1. `repo.rs`: row structs + queries for each table (literal SQL, `query_as`).
2. `github.rs`: authorize URL builder; code exchange
   (`POST https://github.com/login/oauth/access_token`, `Accept: application/json`);
   `GET https://api.github.com/user` → `{id, login, name}`. Add `reqwest`
   (rustls, json) to this crate only. Env: `GITHUB_CLIENT_ID`,
   `GITHUB_CLIENT_SECRET`, `GITHUB_REDIRECT_URI`, `PUBLIC_BASE_URL`.
3. `BeginGithubLogin`: validate `redirect_to` (must start with `/`, no `//`),
   insert state, return URL with `scope=read:user`.
4. `CompleteGithubLogin`: consume state → exchange code → fetch user →
   `identity.EnsureSubject(handle = lower(login), display_name = name)` →
   upsert `github_identities` (`on conflict (github_id) do update set login,
   last_login_at`) → mint session (new family) → `SessionTokens`.
5. `RefreshSession`, `RevokeSession` (by access hash → set `revoked_at` on the
   row; logout does not need to kill the family), `IntrospectToken`.
6. Requesters: `CreateRequester` (`client_id = tokens::random_client_id()`),
   `ListRequesters`, `GetRequester`, `RotateRequesterSecret`,
   `IssueClientToken` (lookup by `client_id`, `digest_eq` the secret, mint).
7. Tests: integration against `TEST_DATABASE_URL` — rotation/reuse revocation,
   expiry, owner check on rotate, introspection of each token kind; unit tests
   for `redirect_to` validation. Mock GitHub with a local `axum` server.

Run: `make run-auth`. Env beyond the common set: `IDENTITY_URL`, the GitHub vars.
