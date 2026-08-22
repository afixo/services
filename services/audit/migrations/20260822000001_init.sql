-- audit-service: an append-only, hash-chained record of every disclosure
-- decision. Rows are inserted by the JetStream consumer only; nothing updates
-- or deletes them (enforced by the trigger below, as defence in depth against
-- the service's own bugs — a database superuser can still drop the trigger,
-- which is exactly what the chain makes detectable).

create table disclosure_events (
    seq            bigserial primary key,
    event_id       uuid not null unique,        -- idempotency key (at-least-once delivery)
    subject_id     uuid not null,
    requester_id   uuid not null,
    purpose        text not null,
    allowed        boolean not null,
    reason         text not null,
    persona_id     uuid,
    persona_label  text,
    rule_id        uuid,
    disclosed_keys text[] not null default '{}',
    withheld_keys  text[] not null default '{}',
    decided_at     timestamptz not null,
    recorded_at    timestamptz not null default now(),
    prev_hash      bytea not null,              -- 32 zero bytes for the genesis row
    hash           bytea not null unique
);

create index disclosure_events_by_subject on disclosure_events (subject_id, seq desc);

create function disclosure_events_immutable() returns trigger language plpgsql as $$
begin
    raise exception 'disclosure_events is append-only';
end $$;

create trigger disclosure_events_no_update
    before update or delete on disclosure_events
    for each row execute function disclosure_events_immutable();
