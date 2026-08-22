-- identity-service owns the people and their partial identities.

create table subjects (
    id           uuid primary key,
    handle       text not null unique,          -- public reference: /v1/disclose/:handle
    display_name text not null default '',
    created_at   timestamptz not null default now()
);

create table personas (
    id         uuid primary key,
    subject_id uuid not null references subjects (id) on delete cascade,
    label      text not null,                   -- "legal", "work", "social"…
    created_at timestamptz not null default now(),
    unique (subject_id, label)
);

create table persona_fields (
    persona_id  uuid not null references personas (id) on delete cascade,
    key         text not null,
    value       text not null,
    sensitivity smallint not null check (sensitivity between 0 and 3),
    updated_at  timestamptz not null default now(),
    primary key (persona_id, key)
);

create index personas_by_subject on personas (subject_id);
