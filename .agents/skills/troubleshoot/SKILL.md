---
name: troubleshoot
description: Diagnose and fix failures in this workspace — cargo xtask arch violations, sqlx offline/compile errors, RLS and role errors, failing database tests, problem+json status surprises, NATS/worker issues. Use when a check, build, test or local run fails and the cause is not obvious.
---

# Troubleshoot

First rule: fix the cause. Never silence the check (`#[allow]`, editing `architecture.toml`/`clippy.toml`, deleting a test).

## Build and compile

| Symptom | Cause | Fix |
|---|---|---|
| `no cached data for this query` / `SQLX_OFFLINE` error | a new or changed `query!` without metadata | `cargo xtask db up && cargo xtask db migrate && cargo xtask db prepare` |
| the same error only in the editor (rust-analyzer `macro-error`) while `cargo check` passes | stale expansion: rust-analyzer expanded the macro before `db prepare` wrote `.sqlx/`, and does not watch that directory | run **rust-analyzer: Restart server** (or edit the file) |
| `query!` type mismatch, e.g. `expected Option<..>` | column nullability inferred by Postgres | fix the schema (`NOT NULL`), or override in SQL: `col AS "col!"` (non-null) or `AS "col?"` (nullable) |
| `relation "x.y" does not exist` while compiling or preparing | migration not applied locally | `cargo xtask db migrate` (or `db reset`) |
| `use of disallowed type axum::Json` | used axum's extractor | `api_kit::Json` (also `Path`, `Query`) |
| `allow attribute ... use expect` | `#[allow]` is banned | `#[expect(lint, reason = "...")]`, only if the code is right |
| `arithmetic_side_effects` in an engine | unchecked `+ - * /` | `checked_add(..).ok_or(MoneyError::OutOfRange)?` etc. |

## `cargo xtask arch` violations

Each message names a rule. The usual fixes:

| Rule | Fix |
|---|---|
| ARCH-002 | the dependency crosses a tier or context. Move the code, or communicate by events. |
| ARCH-004/005 | match the slice shape: workflow in `<feature>.rs`, SQL in a private `mod db` with `pub(super)` items |
| ARCH-009 | add `.routes(utoipa_axum::routes!(<feature>::http::handle))` after `// xtask:routes` in `lib.rs` |
| SEC-002 | an `OpCtx` is being built outside an auth edge: take `&OpCtx` as a parameter instead |
| DB-001/002/003/010 | a context touched pools, `SystemStore`, `set_config`/`SET ROLE`, or runtime `sqlx::query(..)`: use `TenantTx` and `query!` |
| DB-007 | bound the query (`LIMIT`, keyset) or select by `= ANY($1)` |
| DB-011 | migration name, schema qualification, or `sqlx.toml`: use `cargo xtask new migration` |
| DB-013 | SQL references another context's schema: use its events or published API |
| OBS-001 / SEC-003 | the workflow lacks `#[tracing::instrument(skip_all, ..)]` or `ctx: &OpCtx` as its first parameter |
| TEST-005 | the workflow is never called by an integration test: add one |
| SCAF-001 | a `SCAFFOLD:` placeholder remains |

## Database and RLS at runtime or in tests

| Symptom | Cause | Fix |
|---|---|---|
| query returns no rows that should exist | RLS: wrong tenant in the context, or the row has another `tenant_id` | check `ctx.tenant_id()`; never work around it with a superuser |
| `new row violates row-level security policy` | `tenant_id` set explicitly to another tenant, or no tenant bound | let `tenant_id` default, or bind `tx.tenant_id()`; always use `db.begin(ctx)` |
| `invalid input syntax for type uuid: ""` | a policy uses `current_setting(..)::uuid` directly | use `platform.current_tenant_id()` (it applies `NULLIF`) |
| `RoleMisconfigured: role ... is a superuser / owns tables / has BYPASSRLS` | wrong credentials for this store | `APP_DATABASE__URL` → `app_rw`; `APP_SYSTEM_DATABASE__URL` → `app_system`; migrations → `app_owner` |
| `permission denied for table` | missing `GRANT` in the migration | add a minimal grant to `app_rw` in a **new** migration |
| `LockTimeout` / 503 `CONCURRENT_MODIFICATION` | contention; expected under load | the client retries (`Retry-After`); if constant, check for long transactions |
| `#[sqlx::test]` failing instantly | `DATABASE_URL` not set, or database down | run through `cargo xtask test` (it sets local defaults); `cargo xtask db up` |
| checksum mismatch on migrate | a local migration was edited after being applied | `cargo xtask db reset` (local only; never edit merged migrations) |

## HTTP status surprises

| Status | Meaning |
|---|---|
| 400 `IDEMPOTENCY_KEY_REQUIRED` | missing `Idempotency-Key` header |
| 409 `IDEMPOTENCY_REQUEST_IN_FLIGHT` | the same key is still running |
| 412 | stale `If-Match`; GET the resource for its current `ETag` |
| 422 `IDEMPOTENCY_KEY_REUSED` | same key, different body |
| 428 | send `If-Match` (update) or `If-None-Match: *` (create) |
| 404 for an id you just created | different tenant in the token |

Every problem body has `trace_id` and `request_id`: search the logs or traces for them.

## Worker and messaging

| Symptom | Fix |
|---|---|
| events stay in `platform.outbox_events` | the worker is not running, or NATS is unreachable. Check `/health/ready` on :8081 and worker logs for `outbox relay batch failed`. |
| consumer never receives | `EVENT_TYPES` must match the emitted `EVENT_TYPE` exactly (including `.v1`); check that the consumer is registered in `tasks.rs` |
| `invalid consumer name` | `CONSUMER` must be `[a-z0-9_-]` |
| effect applied twice | the effect is not in the provided `tx`, or the handler committed/opened its own transaction |
