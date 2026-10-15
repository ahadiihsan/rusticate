---
name: add-migration
description: Add or change database schema in this workspace — new tables, columns, indexes, constraints, grants, data backfills — safely for multi-tenant RLS and zero-downtime deploys. Use for any change under a crate's migrations/ directory.
---

# Add a migration

Each bounded context owns one PostgreSQL schema, named after it, and its own migration history (DB-011). The platform schema (`platform.*`) belongs to `datastore`.

## Create

```sh
cargo rusticate new migration <ctx> "create shipments"     # → crates/domains/<ctx>/migrations/<timestamp>_create_shipments.sql
```

The file starts from the **tenant-table template**. Keep every element unless the table is genuinely global:

```sql
SET LOCAL lock_timeout = '5s';            -- fail fast instead of queueing behind traffic
SET LOCAL statement_timeout = '5min';

CREATE TABLE <ctx>.shipments (
    tenant_id   uuid        NOT NULL DEFAULT platform.current_tenant_id(),
    id          uuid        NOT NULL,                     -- UUIDv7 from the application
    ...
    version     integer     NOT NULL DEFAULT 1,           -- if clients update it (If-Match)
    created_at  timestamptz NOT NULL DEFAULT now(),
    CONSTRAINT shipments_pkey PRIMARY KEY (tenant_id, id)
);
ALTER TABLE <ctx>.shipments ENABLE ROW LEVEL SECURITY;
ALTER TABLE <ctx>.shipments FORCE ROW LEVEL SECURITY;
CREATE POLICY tenant_isolation ON <ctx>.shipments
    USING (tenant_id = (SELECT platform.current_tenant_id()))
    WITH CHECK (tenant_id = (SELECT platform.current_tenant_id()));
GRANT SELECT, INSERT, UPDATE ON <ctx>.shipments TO app_rw;   -- minimal; never TRUNCATE
```

Then:

```sh
cargo rusticate db migrate      # apply locally
cargo rusticate test -p <ctx>   # the context's schema test runs the schema guard
cargo rusticate db prepare      # once queries use the new columns
```

## Rules the schema guard checks (fails tests if broken)

| Rule | Requirement |
|---|---|
| DB-004a | `ENABLE` **and** `FORCE ROW LEVEL SECURITY` |
| DB-004b/c | exactly the canonical permissive policy above; no other permissive policy (they OR together) |
| DB-004d | `tenant_id NOT NULL` |
| DB-004e | a table without `tenant_id` is declared: `COMMENT ON TABLE <ctx>.x IS 'tenancy:global <why>'` |
| DB-004f | views: `WITH (security_invoker = true)` |
| DB-004g | `SECURITY DEFINER` functions: `COMMENT ON FUNCTION .. IS 'security-definer:reviewed <why>'` + security review |
| DB-005a | foreign keys between tenant tables include `tenant_id` at the same position: `FOREIGN KEY (tenant_id, parent_id) REFERENCES <ctx>.parents (tenant_id, id)` |
| DB-005b | unique indexes include `tenant_id`: `(tenant_id, number)` |
| DB-005c | every table has a primary key |
| DB-002a | `app_rw` never holds `TRUNCATE` (it ignores RLS) |
| DB-002c | never grant anything to `app_system` outside `platform.*` (it bypasses RLS) |
| DB-003a | never redefine `platform.current_tenant_id()`; every policy depends on it |
| DB-011b | nothing in schema `public` |

`cargo rusticate arch` additionally checks the file name, that every object is schema-qualified with **this** context's schema, and that `sqlx.toml` declares it. CI runs squawk on new migrations and fails if a merged migration changed.

## Types

- Money: `bigint` minor units plus a `currency text` checked against `^[A-Z]{3}$` (ENG-003). Never `numeric` for stored amounts, and never `float`.
- Time: `timestamptz`.
- Enumerations: `text` with a `CHECK (status IN (...))`, mapped to a Rust enum (`FromStr`).
- Name constraints explicitly (`<table>_<what>`), because error mapping keys on constraint names.

## Zero-downtime changes (expand → migrate → contract)

The running version and the next version both work against the schema at every step.

| Change | Safe sequence |
|---|---|
| Add a column | Add it nullable or with a default. Deploy code that writes it. Backfill. Then add `NOT NULL` in a later migration. |
| Rename a column | Add the new column, dual-write, backfill, switch reads, then drop the old one in a later release. Never `RENAME` in place. |
| Drop a column | Remove all code use first (release), then drop it in a later migration. |
| Index an existing table | A migration of its own whose first line is `-- no-transaction`, using `CREATE INDEX CONCURRENTLY IF NOT EXISTS`. |
| Add a FK / CHECK to a large table | `ADD CONSTRAINT .. NOT VALID`, then `VALIDATE CONSTRAINT` in a later migration. |
| Tighten a type | Add a new column and migrate the data; never `ALTER TYPE` in place on a hot table. |

## Data backfills on RLS tables

Migrations run as `app_owner`, and `FORCE ROW LEVEL SECURITY` applies to the owner too, so a plain `UPDATE` sees no rows. Inside the migration (one transaction):

```sql
ALTER TABLE <ctx>.t NO FORCE ROW LEVEL SECURITY;
UPDATE <ctx>.t SET new_col = ... WHERE new_col IS NULL;   -- batch large tables across migrations
ALTER TABLE <ctx>.t FORCE ROW LEVEL SECURITY;
```

## Never

- Edit a migration that has been merged. Write a new one.
- Create objects in `public` or in another context's schema.
- Grant `BYPASSRLS`, `TRUNCATE`, or ownership to runtime roles.
- Change a crate's `sqlx.toml` `table-name` after its first deploy.
