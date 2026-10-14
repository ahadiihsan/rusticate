-- =============================================================================
-- Billing bounded context. Every table follows the tenant-table template
-- (SOP DB-004/DB-005), verified by test_kit::schema::assert_schema_invariants:
--   * tenant_id NOT NULL, defaulted from the transaction's tenant
--   * primary and foreign keys lead with tenant_id
--   * ENABLE + FORCE row level security, canonical tenant_isolation policy
--   * explicit, minimal grants to app_rw (never TRUNCATE; DELETE only if needed)
-- Money is BIGINT minor units (SOP ENG-003).
-- =============================================================================

-- Fail fast instead of queueing behind live traffic (DB-011).
SET LOCAL lock_timeout = '5s';
SET LOCAL statement_timeout = '5min';

GRANT USAGE ON SCHEMA billing TO app_rw;

-- -----------------------------------------------------------------------------
-- Per-tenant billing configuration. The server, not the client, decides the
-- currency and tax rate of an invoice.
-- -----------------------------------------------------------------------------
CREATE TABLE billing.settings (
    tenant_id     uuid        NOT NULL DEFAULT platform.current_tenant_id(),
    currency      text        NOT NULL,
    tax_rate_bps  integer     NOT NULL,
    version       integer     NOT NULL DEFAULT 1,
    updated_at    timestamptz NOT NULL DEFAULT now(),
    updated_by    uuid        NOT NULL,
    CONSTRAINT settings_pkey PRIMARY KEY (tenant_id),
    CONSTRAINT settings_currency_format CHECK (currency ~ '^[A-Z]{3}$'),
    CONSTRAINT settings_tax_rate_range CHECK (tax_rate_bps BETWEEN 0 AND 10000)
);

ALTER TABLE billing.settings ENABLE ROW LEVEL SECURITY;
ALTER TABLE billing.settings FORCE ROW LEVEL SECURITY;
CREATE POLICY tenant_isolation ON billing.settings
    USING (tenant_id = (SELECT platform.current_tenant_id()))
    WITH CHECK (tenant_id = (SELECT platform.current_tenant_id()));
GRANT SELECT, INSERT, UPDATE ON billing.settings TO app_rw;

-- -----------------------------------------------------------------------------
-- Invoices. Never deleted: voiding is a state transition.
-- -----------------------------------------------------------------------------
CREATE TABLE billing.invoices (
    tenant_id       uuid        NOT NULL DEFAULT platform.current_tenant_id(),
    id              uuid        NOT NULL,
    customer_email  text        NOT NULL, -- personal data: telemetry::Sensitive in Rust
    currency        text        NOT NULL,
    status          text        NOT NULL,
    subtotal_minor  bigint      NOT NULL,
    tax_rate_bps    integer     NOT NULL,
    tax_minor       bigint      NOT NULL,
    total_minor     bigint      NOT NULL,
    version         integer     NOT NULL DEFAULT 1,
    issued_at       timestamptz NOT NULL DEFAULT now(),
    issued_by       uuid        NOT NULL,
    voided_at       timestamptz,
    void_reason     text,
    CONSTRAINT invoices_pkey PRIMARY KEY (tenant_id, id),
    CONSTRAINT invoices_currency_format CHECK (currency ~ '^[A-Z]{3}$'),
    CONSTRAINT invoices_status_valid CHECK (status IN ('issued', 'paid', 'void')),
    CONSTRAINT invoices_amounts_non_negative CHECK (subtotal_minor >= 0 AND tax_minor >= 0),
    CONSTRAINT invoices_total_conserved CHECK (total_minor = subtotal_minor + tax_minor),
    CONSTRAINT invoices_tax_rate_range CHECK (tax_rate_bps BETWEEN 0 AND 10000),
    CONSTRAINT invoices_void_consistent CHECK ((status = 'void') = (voided_at IS NOT NULL))
);

-- Status-filtered keyset pagination (newest first). Unfiltered listing uses the PK.
-- New table in this migration: a blocking build is safe here.
-- squawk-ignore require-concurrent-index-creation
CREATE INDEX invoices_tenant_status_id_idx ON billing.invoices (tenant_id, status, id DESC);

ALTER TABLE billing.invoices ENABLE ROW LEVEL SECURITY;
ALTER TABLE billing.invoices FORCE ROW LEVEL SECURITY;
CREATE POLICY tenant_isolation ON billing.invoices
    USING (tenant_id = (SELECT platform.current_tenant_id()))
    WITH CHECK (tenant_id = (SELECT platform.current_tenant_id()));
GRANT SELECT, INSERT, UPDATE ON billing.invoices TO app_rw;

CREATE TABLE billing.invoice_lines (
    tenant_id         uuid     NOT NULL DEFAULT platform.current_tenant_id(),
    invoice_id        uuid     NOT NULL,
    line_no           smallint NOT NULL,
    description       text     NOT NULL,
    quantity          integer  NOT NULL,
    unit_price_minor  bigint   NOT NULL,
    line_total_minor  bigint   NOT NULL,
    CONSTRAINT invoice_lines_pkey PRIMARY KEY (tenant_id, invoice_id, line_no),
    -- Composite FK: a line can only reference an invoice of the same tenant
    -- (FK checks bypass RLS, so tenant_id must be part of the key).
    CONSTRAINT invoice_lines_invoice_fkey FOREIGN KEY (tenant_id, invoice_id)
        REFERENCES billing.invoices (tenant_id, id),
    CONSTRAINT invoice_lines_line_no_non_negative CHECK (line_no >= 0),
    CONSTRAINT invoice_lines_description_length CHECK (char_length(description) BETWEEN 1 AND 500),
    CONSTRAINT invoice_lines_quantity_positive CHECK (quantity > 0),
    CONSTRAINT invoice_lines_price_non_negative CHECK (unit_price_minor >= 0)
);

ALTER TABLE billing.invoice_lines ENABLE ROW LEVEL SECURITY;
ALTER TABLE billing.invoice_lines FORCE ROW LEVEL SECURITY;
CREATE POLICY tenant_isolation ON billing.invoice_lines
    USING (tenant_id = (SELECT platform.current_tenant_id()))
    WITH CHECK (tenant_id = (SELECT platform.current_tenant_id()));
GRANT SELECT, INSERT ON billing.invoice_lines TO app_rw;

-- -----------------------------------------------------------------------------
-- Read model maintained asynchronously by the `billing.record_invoice_stats`
-- event consumer (eventually consistent).
-- -----------------------------------------------------------------------------
CREATE TABLE billing.invoice_stats (
    tenant_id           uuid        NOT NULL DEFAULT platform.current_tenant_id(),
    currency            text        NOT NULL,
    issued_count        bigint      NOT NULL DEFAULT 0,
    issued_total_minor  bigint      NOT NULL DEFAULT 0,
    voided_count        bigint      NOT NULL DEFAULT 0,
    voided_total_minor  bigint      NOT NULL DEFAULT 0,
    updated_at          timestamptz NOT NULL DEFAULT now(),
    CONSTRAINT invoice_stats_pkey PRIMARY KEY (tenant_id, currency)
);

ALTER TABLE billing.invoice_stats ENABLE ROW LEVEL SECURITY;
ALTER TABLE billing.invoice_stats FORCE ROW LEVEL SECURITY;
CREATE POLICY tenant_isolation ON billing.invoice_stats
    USING (tenant_id = (SELECT platform.current_tenant_id()))
    WITH CHECK (tenant_id = (SELECT platform.current_tenant_id()));
GRANT SELECT, INSERT, UPDATE ON billing.invoice_stats TO app_rw;
