-- auth-service owns credentials and tokens. It never stores a token or secret
-- in clear: every *_hash column is SHA-256 of the value handed to the caller.

-- Link between a GitHub account and a subject (subjects live in identity-service).
create table github_identities (
    github_id     bigint primary key,
    subject_id    uuid   not null unique,
    login         text   not null,
    linked_at     timestamptz not null default now(),
    last_login_at timestamptz
);

-- Single-use OAuth state nonces (10 min). Deleted on use.
create table oauth_states (
    state       text primary key,
    redirect_to text not null default '/app',
    expires_at  timestamptz not null
);
create index oauth_states_expiry on oauth_states (expires_at);

-- Subject sessions. One row per access/refresh pair; a refresh creates a new
-- row in the same family and marks the old one rotated. Presenting a refresh
-- token that was already rotated is reuse → the whole family is revoked.
create table sessions (
    id                 uuid primary key,
    subject_id         uuid not null,
    family_id          uuid not null,
    access_hash        bytea not null unique,
    refresh_hash       bytea not null unique,
    access_expires_at  timestamptz not null,
    refresh_expires_at timestamptz not null,
    rotated_at         timestamptz,                -- set when this row's refresh token was consumed
    revoked_at         timestamptz,
    created_at         timestamptz not null default now()
);
create index sessions_by_subject on sessions (subject_id);
create index sessions_by_family  on sessions (family_id);
create index sessions_expiry     on sessions (refresh_expires_at);

-- Registered machine clients. Owned by the subject that registered them; only
-- the owner may rotate the secret (fix for FINAL_REPORT §5.4(a)).
create table requesters (
    id               uuid primary key,
    owner_subject_id uuid not null,
    client_id        text not null unique,
    name             text not null,
    secret_hash      bytea not null,
    created_at       timestamptz not null default now(),
    rotated_at       timestamptz
);
create index requesters_by_owner on requesters (owner_subject_id);

-- Opaque requester bearer tokens (1 h). Expiry is a column, enforced on lookup;
-- a periodic cleanup deletes expired rows.
create table requester_tokens (
    token_hash   bytea primary key,
    requester_id uuid not null references requesters (id) on delete cascade,
    expires_at   timestamptz not null,
    created_at   timestamptz not null default now()
);
create index requester_tokens_expiry on requester_tokens (expires_at);
