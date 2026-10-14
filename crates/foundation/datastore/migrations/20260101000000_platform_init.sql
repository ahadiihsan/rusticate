-- =============================================================================
-- Platform primitives shared by every bounded context.
-- Schema `platform` is created by the migrator (see sqlx.toml).
-- Roles app_rw / app_system are the platform role contract (SOP DB-002).
-- =============================================================================

-- Fail fast instead of queueing behind live traffic (DB-011).
SET LOCAL lock_timeout = '5s';
SET LOCAL statement_timeout = '5min';

GRANT USAGE ON SCHEMA platform TO app_rw, app_system;

-- -----------------------------------------------------------------------------
-- DB-003: the ONLY sanctioned way to read the tenant of the current transaction.
--
-- `NULLIF(..., '')`: once a session has set a custom setting, even
-- transaction-locally, `current_setting(name, true)` returns '' (not NULL)
-- after the transaction ends. Without NULLIF a query on a recycled pooled
-- connection would fail with "invalid input syntax for type uuid" instead of
-- simply matching no rows.
-- -----------------------------------------------------------------------------
CREATE FUNCTION platform.current_tenant_id() RETURNS uuid
    LANGUAGE sql STABLE PARALLEL SAFE
    AS $$ SELECT NULLIF(current_setting('app.tenant_id', true), '')::uuid $$;

CREATE FUNCTION platform.current_actor_id() RETURNS uuid
    LANGUAGE sql STABLE PARALLEL SAFE
    AS $$ SELECT NULLIF(current_setting('app.actor_id', true), '')::uuid $$;

REVOKE ALL ON FUNCTION platform.current_tenant_id(), platform.current_actor_id() FROM PUBLIC;
GRANT EXECUTE ON FUNCTION platform.current_tenant_id(), platform.current_actor_id() TO app_rw, app_system;

-- -----------------------------------------------------------------------------
-- MSG-001: transactional outbox. The application role may only INSERT (inside
-- the same transaction as the state change); the relay (app_system) reads and
-- deletes. Rows are deleted once published, so the table stays small and
-- free of update churn (no status column to rewrite).
-- -----------------------------------------------------------------------------
CREATE TABLE platform.outbox_events (
    id              uuid        NOT NULL,
    tenant_id       uuid        NOT NULL DEFAULT platform.current_tenant_id(),
    event_type      text        NOT NULL,
    aggregate_type  text        NOT NULL,
    aggregate_id    uuid        NOT NULL,
    payload         jsonb       NOT NULL,
    correlation_id  uuid        NOT NULL,
    -- W3C trace context of the emitting request: the relay and consumers
    -- continue the same distributed trace.
    traceparent     text,
    occurred_at     timestamptz NOT NULL DEFAULT now(),
    -- PK on id alone: the relay drains in global UUIDv7 (time) order.
    CONSTRAINT outbox_events_pkey PRIMARY KEY (id),
    CONSTRAINT outbox_events_event_type_format
        CHECK (event_type ~ '^[a-z][a-z0-9_]*(\.[a-z][a-z0-9_]*)+\.v[0-9]+$')
);

ALTER TABLE platform.outbox_events ENABLE ROW LEVEL SECURITY;
ALTER TABLE platform.outbox_events FORCE ROW LEVEL SECURITY;
CREATE POLICY tenant_isolation ON platform.outbox_events
    USING (tenant_id = (SELECT platform.current_tenant_id()))
    WITH CHECK (tenant_id = (SELECT platform.current_tenant_id()));

GRANT INSERT ON platform.outbox_events TO app_rw;
GRANT SELECT, DELETE ON platform.outbox_events TO app_system;

-- -----------------------------------------------------------------------------
-- MSG-004: consumer inbox. Exactly-once *effects* on top of at-least-once
-- delivery: a handler records (consumer, event_id) in the same transaction as
-- its effects; a redelivered event finds the row and is skipped.
-- -----------------------------------------------------------------------------
CREATE TABLE platform.inbox_events (
    tenant_id     uuid        NOT NULL DEFAULT platform.current_tenant_id(),
    consumer      text        NOT NULL,
    event_id      uuid        NOT NULL,
    processed_at  timestamptz NOT NULL DEFAULT now(),
    CONSTRAINT inbox_events_pkey PRIMARY KEY (tenant_id, consumer, event_id)
);

-- New table in this migration: a blocking build is safe here.
-- squawk-ignore require-concurrent-index-creation
CREATE INDEX inbox_events_processed_at_idx ON platform.inbox_events (processed_at);

ALTER TABLE platform.inbox_events ENABLE ROW LEVEL SECURITY;
ALTER TABLE platform.inbox_events FORCE ROW LEVEL SECURITY;
CREATE POLICY tenant_isolation ON platform.inbox_events
    USING (tenant_id = (SELECT platform.current_tenant_id()))
    WITH CHECK (tenant_id = (SELECT platform.current_tenant_id()));

GRANT SELECT, INSERT ON platform.inbox_events TO app_rw;
GRANT SELECT, DELETE ON platform.inbox_events TO app_system;

-- -----------------------------------------------------------------------------
-- API-005: idempotency keys. Claimed and completed inside the SAME transaction
-- as the business change: a crash rolls both back (no stuck "in progress"
-- keys), and a concurrent duplicate blocks on the key's row lock until the
-- first request commits, then replays its stored response.
-- -----------------------------------------------------------------------------
CREATE TABLE platform.idempotency_keys (
    tenant_id        uuid        NOT NULL DEFAULT platform.current_tenant_id(),
    actor_id         uuid        NOT NULL,
    operation        text        NOT NULL,
    idempotency_key  text        NOT NULL,
    fingerprint      bytea       NOT NULL,
    response         jsonb,
    created_at       timestamptz NOT NULL DEFAULT now(),
    expires_at       timestamptz NOT NULL,
    CONSTRAINT idempotency_keys_pkey PRIMARY KEY (tenant_id, actor_id, operation, idempotency_key),
    CONSTRAINT idempotency_keys_key_length CHECK (char_length(idempotency_key) BETWEEN 1 AND 255),
    CONSTRAINT idempotency_keys_fingerprint_length CHECK (octet_length(fingerprint) = 32)
);

-- New table in this migration: a blocking build is safe here.
-- squawk-ignore require-concurrent-index-creation
CREATE INDEX idempotency_keys_expires_at_idx ON platform.idempotency_keys (expires_at);

ALTER TABLE platform.idempotency_keys ENABLE ROW LEVEL SECURITY;
ALTER TABLE platform.idempotency_keys FORCE ROW LEVEL SECURITY;
CREATE POLICY tenant_isolation ON platform.idempotency_keys
    USING (tenant_id = (SELECT platform.current_tenant_id()))
    WITH CHECK (tenant_id = (SELECT platform.current_tenant_id()));

GRANT SELECT, INSERT, UPDATE ON platform.idempotency_keys TO app_rw;
GRANT SELECT, DELETE ON platform.idempotency_keys TO app_system;
