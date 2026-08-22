-- policy-service owns the purpose vocabulary and disclosure rules.
-- Cross-service references (subject_id, requester_id, persona_id) are plain
-- uuids: they are validated by RPC at write time, never by foreign key, because
-- the referenced rows live in other services' databases.

create table purposes (
    name        text primary key,
    description text not null,
    sort_order  int  not null default 0
);

-- The closed vocabulary (FINAL_REPORT §3.3). Adding a purpose is a migration,
-- never an API call: purpose limitation is only checkable against a fixed list.
insert into purposes (name, description, sort_order) values
    ('social_display',   'Show this person in a social or community context',          10),
    ('professional',     'Represent this person in a professional or workplace context', 20),
    ('shipping',         'Deliver physical goods to this person',                      30),
    ('billing',          'Invoice or charge this person',                              40),
    ('age_verification', 'Confirm this person meets an age requirement',               50),
    ('legal_kyc',        'Satisfy a legal know-your-customer obligation',              60),
    ('support',          'Provide customer support to this person');

create table disclosure_rules (
    id              uuid primary key,
    subject_id      uuid not null,
    requester_id    uuid,                                   -- null = any requester
    purpose         text references purposes (name),       -- null = any purpose
    persona_id      uuid not null,
    max_sensitivity smallint not null check (max_sensitivity between 0 and 3),
    allow_keys      text[],                                 -- null = ceiling only; '{}' = release nothing
    priority        int  not null default 0,
    created_seq     bigserial not null,                     -- final tie-break in the engine
    created_at      timestamptz not null default now()
);

-- The one query of the algorithm (ListCandidateRules).
create index disclosure_rules_lookup
    on disclosure_rules (subject_id, requester_id, purpose);

create index disclosure_rules_by_subject
    on disclosure_rules (subject_id, created_seq desc);
