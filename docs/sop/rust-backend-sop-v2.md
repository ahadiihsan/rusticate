# Rust Backend Engineering Standard — SOP v2

| | |
|---|---|
| **Status** | Active. Supersedes [SOP v1](../../rust_backend_architecture_engineering_sop.md) (kept unchanged for reference). Changes from v1 and their reasons: [v1-to-v2.md](v1-to-v2.md). |
| **Applies to** | Every multi-tenant backend service in this workspace, written by engineers or coding agents. |
| **Owner** | Platform team (see [`.github/CODEOWNERS`](../../.github/CODEOWNERS)). Changing a rule requires an ADR ([GOV-001](#13-governance-and-supply-chain-gov-sup)). |
| **Stack** | Rust 2024 (toolchain pinned in [`rust-toolchain.toml`](../../rust-toolchain.toml)) · Tokio · Axum 0.8 · Tower · SQLx 0.9 on PostgreSQL 18 (RLS) · NATS JetStream · OpenTelemetry · utoipa (OpenAPI 3.1) |

## 0. How this standard works

v1 was a long document with large code listings that nothing compiled and nothing enforced. v2 has three layers that cannot drift apart:

1. **This document**: why each rule exists, plus the [rule catalogue](#15-rule-catalogue).
2. **The reference implementation**: the [`billing`](../../crates/domains/billing) context and the foundation crates. They compile, are tested, and show how to follow every rule. When this document and the code disagree, the code is right and this document has a bug.
3. **The enforcement**: the compiler, clippy, `cargo xtask arch`, the database schema guard, the test suite, `cargo-deny` and CI. A rule a machine can check *is* checked. The few rules only a reviewer can judge are marked **[review]**.

Keywords follow RFC 2119: **MUST** means a violation fails CI or review; **SHOULD** means deviating requires a written reason in the PR; **MAY** is optional.

**Definition of done** for any change: `cargo xtask verify` passes. CI runs `cargo xtask ci`, the same checks.

### The seven principles

1. **Boundaries by construction.** Crate tiers, capability rules and privacy make the wrong dependency fail to compile or fail `arch`. Convention alone is not a boundary.
2. **Pure engines, imperative orchestrators.** Business rules are pure synchronous functions; workflows orchestrate I/O around them.
3. **Tenant isolation in depth.** `OpCtx` → `TenantTx` → PostgreSQL RLS → catalog schema guard → role check at startup. Each layer assumes the others failed.
4. **Every side effect is transactional.** State, outbox events, idempotency keys and inbox records commit or roll back together. There are no dual writes.
5. **Parse, don't validate.** Raw input becomes typed values at the edge. Money is integers.
6. **Contract first.** Every error is `problem+json`. The OpenAPI document is a committed, reviewed artifact.
7. **What a human must remember, a machine checks.** If a rule matters and can be checked, it is.

---

## 1. Workspace topology (ARCH)

```text
rusticate/
├── Cargo.toml              workspace: shared dependency versions, lints, profiles
├── architecture.toml       machine-readable tiers, allowlists, capabilities (enforced by `cargo xtask arch`)
├── clippy.toml / deny.toml / rustfmt.toml / rust-toolchain.toml / .squawk.toml
├── AGENTS.md               operating manual for coding agents (and engineers)
├── .agents/skills/         task playbooks (agent-agnostic; `.claude/skills` links here)
├── apps/                   TIER app: deployable binaries — wiring only
│   ├── api/                HTTP API: `serve`, `migrate`, `openapi`
│   └── worker/             outbox relay, event consumers, maintenance
├── crates/foundation/      TIER foundation: platform chassis — no business logic
│   ├── telemetry/          logs, traces, W3C propagation, Sensitive<T>
│   ├── api-kit/            problem+json, ApiError, extractors, pagination, preconditions
│   ├── op-context/         OpCtx, ids, permissions, `mint` capability
│   ├── datastore/          DataStore/TenantTx (RLS), SystemStore, outbox, inbox, platform schema
│   ├── idempotency/        Idempotency-Key extractor + same-transaction storage
│   ├── messaging/          outbox relay, consumer runtime, JetStream adapter, retention
│   ├── server-kit/         middleware stack, health probes, graceful shutdown, docs
│   └── test-kit/           TIER testing: role-accurate test DBs, contexts, schema guard
├── crates/domains/         TIER domain / engine: bounded contexts
│   ├── iam/                shared context: bearer token → OpCtx
│   ├── billing/            reference context (feature slices)
│   └── billing-engine/     pure rules of billing
├── xtask/                  TIER tooling: enforcement, scaffolding, automation
├── docker/ docker-compose.yml   local Postgres (with the role contract) and NATS
├── deploy/k8s/             reference manifests: migrate Job, api, worker
└── docs/                   this standard, ADRs, configuration, operations
```

| Tier | May depend on (normal deps) | Notes |
|---|---|---|
| app | foundation, domain, engine | Composition root only (ARCH-008). |
| domain | foundation, **its own** `<name>-engine`, shared domains (`[domains].shared`) | Other contexts are reached through events (MSG-*). |
| engine | **allowlist**: `serde`, `thiserror` (dev: `proptest`, `rust_decimal`) | No workspace crates, no I/O, no async. |
| foundation | foundation | No domain knowledge. |
| testing | foundation | `test-kit`: `[dev-dependencies]` only. |
| tooling | anything | `xtask`: not shipped, not constrained. |

- **ARCH-001 MUST** — Nothing depends on tooling crates. *Enforced: `arch`.*
- **ARCH-002 MUST** — Dependencies follow the tier table above, as declared in [`architecture.toml`](../../architecture.toml). *Enforced: `arch`.*
- **ARCH-003 MUST** — `test-kit` appears only under `[dev-dependencies]`. *Enforced: `arch`.*
- **ARCH-007 MUST** — A context exposes `router<S>()` generic over the host state (`DataStore: FromRef<S>`), so any product binary can mount it. *Enforced: compiler (the API mounts it generically).*
- **ARCH-008 MUST** — `apps/*` contain configuration, wiring and routing only: no business logic, no SQL. *[review]*

## 2. The feature slice (ARCH-004..ARCH-006, ARCH-009)

Code inside a context is organised **by use case**, named `<verb>_<noun>` in the ubiquitous language. There are never horizontal folders such as `controllers/` or `repositories/`.

```text
crates/domains/billing/src/
├── lib.rs                     public facade: router(), MIGRATOR, permissions, events, entry points
├── error.rs                   BillingError + its single HTTP mapping
├── issue_invoice.rs           HOP 2 workflow
└── issue_invoice/
    ├── http.rs                HOP 1 adapter: DTOs, TryFrom → command, handler, OpenAPI
    └── db.rs                  HOP 3 private SQL (`query!` family)
```

| Hop | File | Does | Never |
|---|---|---|---|
| 1 | `<feature>/http.rs` (or `handler.rs` for events) | Extract `OpCtx`/headers/body, parse DTO into a command via `TryFrom`, call the workflow, render the DTO | Business decisions, SQL |
| 2 | `<feature>.rs` | `ctx.require(..)` first; open the tenant tx; claim idempotency; load via `db::*`; decide with the engine; persist; `tx.emit(..)`; commit | Raw pools, HTTP types, inline business rules |
| 3 | `<feature>/db.rs` + `<context>-engine` | Compile-time checked SQL against `&mut TenantTx`; pure rule functions | Branching on business state (db); I/O (engine) |

Read [`issue_invoice.rs`](../../crates/domains/billing/src/issue_invoice.rs) first: it is the canonical command workflow and exercises every rule in sections 3–8.

- **ARCH-004 MUST** — Each feature has `<feature>.rs`, `<feature>/db.rs` and an adapter (`http.rs` or `handler.rs`). *Enforced: `arch`; generated by `cargo xtask new feature`.*
- **ARCH-005 MUST** — `mod db;` is private and its items are `pub(super)`. No other feature or context can run another feature's SQL. *Enforced: `arch`, compiler.*
- **ARCH-006 MUST** — Every feature has a `pub async fn` entry workflow taking `ctx: &OpCtx` first (`(ctx, db, cmd)` for HTTP, `(ctx, tx, input)` for consumers).
  - Composition inside one context goes through `pub(crate) fn *_in_tx(tx, ..)` helpers. These perform no permission check, because their caller already did; see [`get_invoice::load_in_tx`](../../crates/domains/billing/src/get_invoice.rs).
  - Composition across contexts is by events only.

  *Enforced: `arch` (entry shape); ARCH-002 (no cross-context deps).*
- **ARCH-009 MUST** — Every `<feature>/http.rs` handler is registered in `router()`. *Enforced: `arch`.*

## 3. Identity, authorization and secrets (SEC)

- **SEC-001 MUST** — Every workflow and every database transaction is driven by an [`OpCtx`](../../crates/foundation/op-context/src/context.rs). It carries tenant, actor, actor kind, correlation id and granted permissions. It is cheap to clone (one `Arc`), and its `Debug` output hides the permission list. *Enforced: compiler (`DataStore::begin` requires `&OpCtx`).*
- **SEC-002 MUST** — Only authentication edges may construct an `OpCtx` (the `op_context::mint` capability): IAM middleware after verifying a token, the consumer runtime from a broker envelope, and tests. *Enforced: `arch` (`[capabilities.mint]`).*
- **SEC-003 MUST** — The first statement of every entry workflow is `ctx.require(PERMISSION)?`. *Enforced: `arch` (first parameter); [review] (first statement); tests (`*_requires_its_permission`).*
- **SEC-004 MUST** — Permissions are constants named `<context>.<resource>.<action>`, declared in the context's `permissions.rs`.
  - There is no implicit "admin can do everything" in code. Roles are expanded into explicit permissions when the identity provider issues the token, so four-eyes rules cannot be bypassed by a code path.

  *Enforced: [review]; the scaffolder generates the convention.*
- **SEC-005 MUST** — Token minting (`iam/dev-tokens`) never reaches a deployable binary. *Enforced: `arch` (`[[forbidden_features]]`).*
- **SEC-006 MUST** — Personal data and secrets are wrapped in [`telemetry::Sensitive<T>`](../../crates/foundation/telemetry/src/sensitive.rs) (or `secrecy::SecretString` for credentials).
  - `Debug` prints `[REDACTED]`, and there is no `Display`.
  - Access is through the explicit, greppable `.expose()`.

  *Enforced: [review]; the types make the safe path the easy one.*
- **SEC-007 MUST** — Outside local development, tokens are verified with a public key (RS256/ES256). The HS256 shared secret is refused at startup, and the algorithm is pinned (`none` is impossible). *Enforced: [`api` config validation](../../apps/api/src/config.rs), tests.*
- **SEC-008 MUST** — Another tenant's resource is indistinguishable from a missing one (404, never 403). Ids leak nothing. *Enforced: RLS makes it the natural outcome; tests.*

## 4. Data, tenancy and PostgreSQL (DB, ID)

### 4.1 The role contract

| Role | Used by | Attributes | Can |
|---|---|---|---|
| `app_owner` | migrations, `#[sqlx::test]`, `db prepare` | not superuser, no `BYPASSRLS` | own schemas and tables |
| `app_rw` | api, event consumers | not superuser, **no `BYPASSRLS`**, owns nothing | the explicit grants per table; never `TRUNCATE` |
| `app_system` | outbox relay, maintenance | `BYPASSRLS`, owns nothing | grants on `platform.*` only |

- **DB-001 MUST** — Domain crates never touch connection pools. *Enforced: `arch` (`[capabilities.pool]`).*
- **DB-002 MUST** — Services connect with the role their store expects, and `app_system` holds privileges only on `platform.*` (it bypasses RLS, so it must never be able to read a context's tables).
  - `DataStore::connect` and `SystemStore::connect` verify the role at startup and refuse a superuser, an owner, or the wrong `BYPASSRLS` setting.
  - A misconfigured connection string would otherwise silently disable RLS.

  *Enforced: [startup posture check](../../crates/foundation/datastore/src/posture.rs), schema guard (DB-002a/b/c), tests.*

### 4.2 Tenancy

- **DB-003 MUST** — The tenant of a transaction is set only by `DataStore::begin`/`begin_read_only`, using `set_config(.., is_local => true)`. It vanishes at commit or rollback and can never leak to the next borrower of a pooled connection.
  - SQL reads it only through `platform.current_tenant_id()`.
  - That function uses `NULLIF(.., '')`: after a transaction-local setting ends, Postgres returns `''`, not NULL.
  - Its definition is pinned: every policy depends on it, so no migration may redefine it.

  *Enforced: `arch` (`[capabilities.session_settings]`, migration ownership), schema guard (DB-003a), [tests](../../crates/foundation/datastore/tests/datastore.rs).*
- **DB-004 MUST** — Every table with a `tenant_id` column follows the tenant-table template (see the [billing migration](../../crates/domains/billing/migrations/20260102000000_billing_init.sql)):
  - `tenant_id uuid NOT NULL DEFAULT platform.current_tenant_id()`;
  - `ENABLE` and `FORCE ROW LEVEL SECURITY`;
  - exactly one **permissive** `FOR ALL` policy using the canonical predicate `tenant_id = (SELECT platform.current_tenant_id())` in both `USING` and `WITH CHECK`. Permissive policies are OR-ed, so any other permissive policy would widen access. A restrictive policy alone denies everything, which was v1's bug;
  - a table without `tenant_id` is declared `COMMENT ON TABLE .. IS 'tenancy:global <reason>'`;
  - views set `security_invoker = true`;
  - every `SECURITY DEFINER` function is commented `security-definer:reviewed <reason>`.

  *Enforced: [schema guard](../../crates/foundation/test-kit/src/schema.rs) (DB-004a–g), run by every context's tests and by the product test.*
- **DB-005 MUST** — Keys lead with `tenant_id`:
  - primary keys are `(tenant_id, id)`;
  - foreign keys between tenant tables include `tenant_id` at the matching position, because FK checks bypass RLS and a plain FK lets tenant A reference tenant B's row;
  - unique indexes include `tenant_id`, so uniqueness cannot leak existence across tenants;
  - every table has a primary key.

  *Enforced: schema guard (DB-005a–c).*
- **DB-013 MUST** — A context's SQL references only its own schema. Platform tables are reached through foundation APIs (`emit`, `record_inbox`, `idempotency::claim`). *Enforced: `arch`.*
- **ID-001 MUST** — Identifiers are UUIDv7 (`Uuid::now_v7()`), generated by the application. *Enforced: `arch`; `uuid` is built without `v4`.*

### 4.3 Transactions and queries

- **DB-006 MUST** — Query-only workflows use `begin_read_only` (`BEGIN READ ONLY`, one round trip). Postgres then rejects accidental writes, and the transaction can later be routed to a replica. *Enforced: [review]; the query scaffold uses it.*
- **DB-007 MUST** — No unbounded reads.
  - `fetch_all` is only used on queries with `LIMIT` or `= ANY(..)`.
  - Lists page by keyset (`id < $cursor ORDER BY id DESC LIMIT n+1`) through [`api_kit::pagination`](../../crates/foundation/api-kit/src/pagination.rs), never by `OFFSET`.
  - Batch jobs walk keyset pages in short transactions. They never hold one long cursor transaction: it pins a pooled connection and the xmin horizon (vacuum stalls) and cannot resume after a crash.

  *Enforced: `arch` (unbounded `fetch_all`).*
- **DB-008 MUST** — Concurrent updates are safe.
  - Mutable aggregates carry `version integer`. Updates are guarded with `WHERE version = $expected`, and zero rows means stale (412 over HTTP, API-007). Optimistic concurrency is the default; see [`void_invoice`](../../crates/domains/billing/src/void_invoice.rs).
  - Use `SELECT .. FOR UPDATE` only for contended counters or scarce stock, locking multiple rows in ascending key order.

  *Enforced: tests (`concurrent_voids_exactly_one_wins`).*
- **DB-009 MUST** — Database errors are classified by SQLSTATE in [`DataStoreError`](../../crates/foundation/datastore/src/error.rs):
  - serialization failure, deadlock, lock timeout, statement timeout, pool exhaustion, and server failover or restart (class `08`, `57P01`–`57P03`, `25P03`, `53300`) are **transient** and become 503 + `Retry-After` (consumers retry them);
  - constraint violations map to domain errors, or to logged 500s when validation should have caught them.

  *Enforced: tests.*
- **DB-010 MUST** — All application SQL uses the compile-time-checked `query!` family.
  - The `.sqlx/` metadata is committed, and builds are offline (`SQLX_OFFLINE=true` in [`.cargo/config.toml`](../../.cargo/config.toml)).
  - After changing a query, run `cargo xtask db prepare`.

  *Enforced: `arch` (`[capabilities.raw_sql]`), `cargo xtask ci` (`db prepare --check`).*

### 4.4 Schema ownership and migrations

- **DB-011 MUST** — Each bounded context owns one schema, named after the context, and its migrations.
  - Its `sqlx.toml` sets `create-schemas = ["<schema>"]` and `table-name = "<schema>._sqlx_migrations"`, so a context can be mounted into any product database with its own history.
  - Every migration:
    - is named `<YYYYMMDDHHMMSS>_<description>.sql`;
    - schema-qualifies every object, in its own schema only. It creates, alters or grants on nothing elsewhere and references no other schema, except calling `platform.current_tenant_id()`/`current_actor_id()`. Nothing lives in `public`;
    - starts with `SET LOCAL lock_timeout` and `statement_timeout`;
    - passes squawk.
  - Merged migrations are **immutable**: fix forward.
  - Changes follow **expand → migrate → contract** so the running version and the next one both work. Indexes on existing tables use `CREATE INDEX CONCURRENTLY` in their own migration (`-- no-transaction`).

  *Enforced: `arch` (naming, ownership, qualification, cross-schema references), schema guard (DB-011b), CI `migrations` job (immutability, squawk).*
- **DB-012 MUST** — Each app lists every schema-owning dependency in `MIGRATORS`, platform first.
  - Migrations run as a separate step with the owner role ([`migrate` Job](../../deploy/k8s/migrate-job.yaml)), never from the serving process.
  - A data backfill on a FORCE-RLS table temporarily `ALTER TABLE .. NO FORCE ROW LEVEL SECURITY` inside the migration transaction, then restores it.

  *Enforced: `arch`, the product schema test.*

## 5. Engines: pure business rules (ENG)

- **ENG-001 MUST** — An engine crate is pure.
  - It depends only on the allowlist in `architecture.toml`.
  - It has no `async`, no `.await`, and no I/O types.

  *Enforced: `arch` (dependency allowlist, async/await), engine `clippy.toml` (I/O types).*
- **ENG-002 MUST** — Engines are deterministic. Time, identifiers and randomness are inputs, never read inside the engine.
  - The engine's own [`clippy.toml`](../../crates/domains/billing-engine/clippy.toml) bans clocks, `now_v7`, and `HashMap`/`HashSet`.
  - Clippy reads only the nearest `clippy.toml`, so it repeats every root entry.

  *Enforced: clippy; `arch` (the mirror is complete).*
- **ENG-003 MUST** — Money is `i64` **minor units** with an explicit [`Currency`](../../crates/domains/billing-engine/src/money.rs) (ISO 4217).
  - Storage is `BIGINT`; the JSON API uses integers named `*_minor`.
  - Every amount is within ±(2⁵³−1), so no JSON client loses precision.
  - Rates are basis points, applied once (e.g. tax on the subtotal) with half-to-even rounding.
  - There are no floats anywhere, and no decimal parsing from JSON numbers.

  *Enforced: `arch` (no `f32`/`f64` in engines), workspace clippy `float_arithmetic`, the types.*
- **ENG-004 MUST** — Overflow is never silent.
  - Engines `#![deny(clippy::arithmetic_side_effects, clippy::as_conversions)]`, so arithmetic goes through `checked_*`/`try_from`.
  - Release builds keep `overflow-checks = true`.

  *Enforced: `arch` (attributes present), clippy, `Cargo.toml` profile.*
- **ENG-005 MUST** — Every rule has example tests and property tests. Properties compare against an **independent oracle** (e.g. `rust_decimal` for rounding) or an **algebraic law** (conservation, order independence, bounds, totality). They never restate the implementation, which was v1's tautological property. See [`invoice.rs` tests](../../crates/domains/billing-engine/src/invoice.rs). *Enforced: [review]; the template demands it.*
- **ENG-006 MUST** — Lifecycles are enums with an exhaustive transition function and no wildcard arm, so a new state or action fails to compile until every transition is decided. *Enforced: compiler; [review] (no `_ =>`).*

## 6. HTTP API contract (API)

- **API-001 MUST** — Every error response is RFC 9457 `application/problem+json`, produced by [`ApiError`](../../crates/foundation/api-kit/src/error.rs) (`type`, `title`, `status`, `detail`, `instance`, `error_code`, `trace_id`, `request_id`, `errors[]`).
  - This includes framework rejections, unknown routes, timeouts, overload and panics.
  - The `trace_id` is the W3C trace id of the request's span, so support can find it.
  - 5xx details are logged with their source chain and never returned.

  *Enforced: [server-kit middleware](../../crates/foundation/server-kit/src/middleware.rs), tests.*
- **API-002 MUST** — Status semantics:

  | Status | Meaning |
  |---|---|
  | 400 | malformed or invalid input (`VALIDATION_FAILED` with every field violation) |
  | 401 | no or invalid credentials |
  | 403 | missing permission |
  | 404 | absent, or another tenant's |
  | 409 | conflicting state or in-flight idempotent request |
  | 412 | stale `If-Match` |
  | 422 | well-formed but violates a business rule, or idempotency key reused with another body |
  | 428 | `If-Match` required |
  | 503 + `Retry-After` | transient (contention, timeout, overload) |

  *Enforced: tests.*
- **API-003 MUST** — Handlers use the problem-aware extractors `api_kit::{Json, Path, Query}`, whose rejections are `problem+json`. Request DTOs named `*Request` use `#[serde(deny_unknown_fields)]`. *Enforced: clippy `disallowed-types` (axum originals), `arch`.*
- **API-004 MUST** — The OpenAPI 3.1 document ([`apps/api/openapi.json`](../../apps/api/openapi.json)) is generated from code and committed.
  - Every operation has an explicit camelCase `operationId` and a `security` declaration.
  - Regenerate it with `cargo xtask openapi` and review the diff like code.

  *Enforced: contract test, `cargo xtask ci`.*
- **API-005 MUST** — Mutations with external effects (money, stock, messages, jobs) require an `Idempotency-Key`. The key is claimed and completed **in the same transaction** as the change ([`idempotency`](../../crates/foundation/idempotency/src/lib.rs)):
  - a failed attempt leaves no key behind;
  - a concurrent duplicate waits on the key row, then replays the original response (or gets 409 after `lock_timeout`);
  - a reused key with another body gets 422.

  Fingerprints hash the *parsed command* as canonical JSON. A replay returns the response as first completed, not the resource's current state (clients `GET` the `Location` for that). *Enforced: the extractor (missing header → 400), [tests](../../crates/foundation/idempotency/tests/idempotency.rs).*
- **API-006 MUST** — Lists use cursor pagination (`limit` 1..=100, opaque `cursor`, `next_cursor`). *Enforced: types.*
- **API-007 MUST** — Mutable resources expose `ETag: "<version>"`. Updates require `If-Match` (428 without it, 412 when stale). Singleton creation uses `If-None-Match: *`. *Enforced: [`Precondition`](../../crates/foundation/api-kit/src/precondition.rs), tests.*
- **API-008 SHOULD** — Paths are `/v<major>/<plural-resource>[/{id}[/<action>]]`. `201 Created` carries `Location`. Breaking changes ship as a new major path, never in place. *[review]*

## 7. Messaging (MSG)

```text
workflow ──(same tx)──▶ platform.outbox_events ──relay──▶ JetStream ──▶ consumer ──(same tx)──▶ effects + platform.inbox_events
```

- **MSG-001 MUST** — Events are recorded with `tx.emit(&event)` in the same transaction as the state change. *Enforced: the only API that publishes is `emit`.*
- **MSG-002 MUST** — Never publish to a broker from a workflow. The [relay](../../crates/foundation/messaging/src/relay.rs):
  - drains the outbox in id order under a transaction-scoped advisory lock, so there is one active relay and the rest are hot standby;
  - publishes with `Nats-Msg-Id = event id`, pipelining the acks;
  - deletes only after every ack, so a crash republishes and deduplication absorbs it;
  - bounds each publish by a timeout below the system pool's `idle_in_transaction_session_timeout`, so a slow broker fails the batch for a retry instead of Postgres killing the session mid-transaction.

  *Enforced: tests (`a_failed_publish_loses_nothing`, pipeline e2e).*
- **MSG-003 MUST** — An event type implements `DomainEvent` with `EVENT_TYPE = "<context>.<aggregate>.<past_tense>.v<N>"` (a database `CHECK` enforces the format). A serialized event is at most 64 KiB (`emit` rejects larger ones), so every outbox row is publishable and one row can never stall the relay. Published versions are immutable: a breaking change is `v<N+1>`, emitted alongside the old one until consumers migrate. *Enforced: DB constraint, [review].*
- **MSG-004 MUST** — Consumers implement [`EventHandler`](../../crates/foundation/messaging/src/consumer.rs):
  - `CONSUMER` is `<context>-<handler>` (`[a-z0-9_-]`, JetStream-safe, validated);
  - their effects are idempotent and order-tolerant;
  - they run inside the runtime's tenant transaction, after `record_inbox` dedup (exactly-once *effects*);
  - each is registered with one `spawn_consumer` line in the worker.

  *Enforced: runtime validation, tests (`redelivered_events_take_effect_once`).*
- **MSG-005 MUST** — Broker credentials allow only the relay to publish to `events.>`. The consumer runtime trusts the envelope's tenant. *[review / infrastructure]*
- **MSG-006 MUST** — Event payloads carry no personal data. They fan out widely and are retained by the broker. *[review]*
- **MSG-007 MUST** — A handler error is either `Transient` (exponential-backoff redelivery, up to 10 attempts) or `Permanent` (terminated, alerted via JetStream advisories). An undecodable payload is permanent. The last failed attempt is terminated explicitly and logged as dead-lettered, never silently dropped. *Enforced: types, tests.*
- **MSG-008 MUST** — The `traceparent` of the emitting request is stored with the outbox row, forwarded as a header, and restored by the consumer: one trace runs from HTTP request to every reaction. *Enforced: implementation.*

## 8. Observability and operations (OBS, OPS, CFG)

- **OBS-001 MUST** — Every entry workflow has `#[tracing::instrument(name = "<context>.<feature>", skip_all, fields(tenant_id, actor_id, ..))]`. `skip_all` keeps arguments, including personal data, out of spans. *Enforced: `arch`.*
- **OBS-002 MUST** — Logs are structured (JSON in production) through `tracing` only, written by a non-blocking writer. *Enforced: clippy (`print_*`, `dbg_macro` denied).*
- **OBS-003 MUST** — The request span carries `request_id`, `tenant_id` and `actor_id`. Clients receive `x-request-id` and `trace_id`. *Enforced: implementation, tests.*
- **OBS-004 MUST** — W3C trace context is continued from inbound `traceparent`, exported via OTLP, and sampled parent-based. *Enforced: implementation.*
- **OBS-005 MUST** — Log levels: 4xx are `debug`, transient 5xx are `warn`, internal errors are `error` with the full source chain. Panics are logged through the panic hook. *Enforced: implementation.*
- **CFG-001 MUST** — Configuration is read once at startup into a typed struct from `APP_*` variables (`__` separates nesting), with `deny_unknown_fields`. Nothing else reads the environment. *Enforced: clippy (`std::env::var` banned).*
- **CFG-002 MUST** — Invalid configuration fails at startup with every problem listed. *Enforced: [config validation](../../apps/api/src/config.rs), tests.*
- **CFG-003 MUST** — Secrets come from the secret manager into the environment; there are no `.env` files in deployed environments. Local defaults live in one place, [`xtask/src/dev_env.rs`](../../xtask/src/dev_env.rs). *[review]*
- **OPS-002 MUST** — `/health/live` checks only the process. `/health/ready` checks dependencies and turns 503 when draining. *Enforced: implementation, tests.*
- **OPS-003 MUST** — Graceful shutdown on SIGTERM proceeds as follows. *Enforced: [server-kit](../../crates/foundation/server-kit/src/shutdown.rs), config validation.*
  1. Readiness turns 503 while the listener keeps serving for `drain_delay` (5 s), so load balancers deregister the instance.
  2. The listener closes and in-flight requests finish, for up to `shutdown_timeout` (20 s).
  3. Background tasks finish their current batch.

  `terminationGracePeriodSeconds` must exceed the sum.
- **OPS-004 MUST** — Platform tables have retention: expired idempotency keys, and inbox rows older than 14 days (longer than the broker retains messages). Rows are purged in bounded batches by the worker. *Enforced: implementation, tests.*
- **OPS-005 MUST** — Everything has a timeout:
  - request deadline;
  - concurrency cap with load shedding;
  - pool acquire timeout;
  - per-session `statement_timeout`, `lock_timeout` and `idle_in_transaction_session_timeout`, sent in the startup packet and also set per role.

  *Enforced: implementation, tests.*
- **OPS-006 MUST** — Images are distroless, non-root, with a read-only root filesystem and all capabilities dropped. They are built from the [`Dockerfile`](../../Dockerfile) with cached dependency layers. *Enforced: CI `image` job, [manifests](../../deploy/k8s).*

## 9. Rust code quality (LINT, ERR, PERF)

- **LINT-001 MUST** — Every product crate sets `[lints] workspace = true`. The workspace denies `unsafe`, `unwrap`/`expect`/`panic`/`todo`/`unreachable`, indexing and string slicing, `float_arithmetic`, `print_*`/`dbg!`, and `std::process::exit`. Pedantic is on, and `-D warnings` in CI. *Enforced: `arch`, clippy.*
- **LINT-002 MUST** — No `#[allow(..)]`. A justified exception is `#[expect(lint, reason = "..")]`, which also fails when it stops being needed. *Enforced: clippy (`allow_attributes`, `allow_attributes_without_reason`).*
- **ERR-001 MUST** — Errors are layered:
  - libraries use `thiserror` enums;
  - each context has one error enum, which workflows return;
  - its HTTP mapping lives in one `From<_> for ApiError`, and its retry classification in one `From<_> for HandlerError`;
  - `anyhow` appears only in binaries and tooling.

  *Enforced: [review]; templates.*
- **ERR-002 MUST** — Production paths do not panic. Fallible operations return `Result`; tests may unwrap. *Enforced: clippy.*
- **PERF-001 SHOULD** — Binaries use mimalloc. The release profile uses thin LTO, `codegen-units = 1`, `panic = "unwind"` (required by the catch-panic layer) and line-table debug info. *Enforced: `Cargo.toml`.*
- **PERF-002 SHOULD** — Pools skip the per-acquire liveness ping (`test_before_acquire(false)`). Session settings travel in the startup packet. Read-only transactions start with one `BEGIN READ ONLY`. *Enforced: implementation.*
- **PERF-003 MUST** — Writes of N rows are one statement (`INSERT .. SELECT FROM UNNEST(..)`), and reads of N related rows use `= ANY($1)`. Never one query per item (N+1). *[review]; see [`issue_invoice/db.rs`](../../crates/domains/billing/src/issue_invoice/db.rs).*
- **PERF-004 SHOULD** — Shared request-scoped data is `Arc`-backed (`OpCtx`). Large values are borrowed, not cloned. *[review]*
- **PERF-005 MUST** — Hot queries do not use `($n IS NULL OR col = $n)`, because Postgres's cached generic plans cannot drop the OR and stop using indexes. Write one query per filter shape, with sentinel bounds (e.g. `Uuid::max()` as the first-page cursor). *[review]; see [`list_invoices/db.rs`](../../crates/domains/billing/src/list_invoices/db.rs).*

## 10. Testing (TEST)

| Level | Where | What | Speed |
|---|---|---|---|
| Engine | `#[cfg(test)]` in `*-engine` | examples + properties (ENG-005) | milliseconds |
| Context integration | `crates/domains/<ctx>/tests/integration/` (one binary) | each workflow on a fresh migrated DB **as `app_rw`**: happy path, every failure, permission, tenant isolation, idempotent replay, concurrency | ~1 s each, parallel |
| Foundation | `crates/foundation/*/tests/` | security properties of the platform (RLS, role posture, relay, dedup) | ~1 s |
| Product | `apps/api/tests/api/` | real router and middleware over HTTP, OpenAPI contract, schema guard over all migrators | ~1 s |
| Pipeline | `apps/worker/tests/worker/` | workflow → outbox → relay → JetStream → consumer → read model | ~1 s |

- **TEST-001 MUST** — Tenant isolation is tested as the runtime role, never as a superuser (superusers bypass RLS, which made v1's isolation test meaningless). *Enforced: `test_kit::TestDb` provides only role-accurate handles.*
- **TEST-002 MUST** — Each crate's integration tests are one binary (`tests/<name>/main.rs` + modules). It links once and shares fixtures. *Enforced: `arch` for contexts.*
- **TEST-003 MUST** — Each product has HTTP-level tests through the production router (`app::build`). *[review]*
- **TEST-004 MUST** — Each product runs `assert_schema_invariants` over all of its migrators. Each context runs it over its own. *Enforced: present in API and scaffolded contexts.*
- **TEST-005 MUST** — Every entry workflow is called by at least one integration test. *Enforced: `arch`.*
- **TEST-006 MUST** — Line coverage of product crates is ≥ 80 % (`cargo xtask coverage`). *Enforced: CI.*
- **TEST-007 SHOULD** — Tests follow Arrange–Act–Assert, with names stating the behaviour (`a_stale_version_is_rejected`). *[review]*

## 11. The workflow: from PRD/TRD to merged code

Humans and agents follow the same playbook ([`.agents/skills/implement-feature`](../../.agents/skills/implement-feature/SKILL.md)):

1. **Classify.**
   - Which context owns the use case? New context: `cargo xtask new domain <name>`, which needs an ADR if it changes the context map.
   - Command (POST), query (GET) or reaction (event)?
   - Does it carry business rules, and so engine work?
   - What data volume is involved: page it, or run it as a batch job?
2. **Schema.** `cargo xtask new migration <ctx> "<what>"`, following the tenant-table template, then `cargo xtask db migrate`.
3. **Rules.** Pure functions and their error variants in `<ctx>-engine`, with example and property tests (ENG-005).
4. **Slice.** `cargo xtask new feature <ctx> <verb_noun> --kind command|query|event`, then replace every `SCAFFOLD:`.
   - Validated command types first.
   - Then `db.rs` queries, then `cargo xtask db prepare`.
   - Then the workflow, then the adapter.
5. **Tests.** Happy path, each failure mode, permission, tenant isolation, and replay or concurrency where relevant.
6. **Contract.** `cargo xtask openapi`, then review the diff.
7. **Done.** `cargo xtask verify` passes and the PR template's checklist is honest.

## 12. Commands

| Command | Purpose |
|---|---|
| `cargo xtask setup` | git hooks + tool check (once) |
| `cargo xtask db up` / `db reset` / `db migrate` / `db prepare [--check]` | local Postgres + NATS, migrations, query metadata |
| `cargo xtask new domain\|feature\|migration ..` | scaffolding that already passes every structural rule |
| `cargo xtask verify [--fast]` | definition of done (`--fast`: no database) |
| `cargo xtask arch` | architecture conformance only |
| `cargo xtask test [-p crate]` / `coverage` | nextest with local defaults / coverage floor |
| `cargo xtask openapi [--check]` | regenerate / verify the API contract |
| `cargo xtask run api\|worker` / `token --perm ..` | run locally / mint a dev token |
| `cargo xtask ci` | everything CI runs |

## 13. Governance and supply chain (GOV, SUP)

- **GOV-001 MUST** — Changing a rule, `architecture.toml`, a lint or an allowlist requires an ADR in [`docs/adr/`](../adr/) and platform review. *Enforced: CODEOWNERS.*
- **GOV-002 MUST** — Code ownership is declared in [CODEOWNERS](../../.github/CODEOWNERS). Migrations always get a database-literate reviewer. *Enforced: branch protection.*
- **SUP-001 MUST** — No known vulnerability ships (RustSec via `cargo deny`). An exception names the advisory, the reason and an expiry. *Enforced: CI.*
- **SUP-002 MUST** — Licenses come from the [allowlist](../../deny.toml); no copyleft. *Enforced: CI.*
- **SUP-003 MUST** — TLS is rustls-only (`openssl`/`native-tls` banned). Crates come only from crates.io. *Enforced: CI.*
- **SUP-004 MUST** — The toolchain is pinned. Dependencies are versioned in the workspace and updated by small reviewed PRs (Dependabot). *Enforced: `rust-toolchain.toml`, CI.*

## 14. Escape hatches

Rules are strict so that exceptions are visible, not impossible:

| Need | Legitimate route |
|---|---|
| A lint fires on correct code | `#[expect(clippy::x, reason = "...")]` at the narrowest scope |
| A context must call another synchronously | add it to `[domains].shared` via ADR (published language), or use events |
| A query needs dynamic SQL | use the [`raw_sql` capability](../../architecture.toml) in a reviewed foundation crate, never in a context |
| A global (non-tenant) table | `COMMENT ON TABLE .. IS 'tenancy:global <reason>'` + review |
| A `SECURITY DEFINER` function | `COMMENT ON FUNCTION .. IS 'security-definer:reviewed <reason>'` + security review |
| A vulnerable dependency without a fix | `deny.toml` `ignore` entry with reason and expiry |

Never weaken `architecture.toml`, `clippy.toml`, `deny.toml` or a policy to make a change pass. That is the change that needs an ADR.

## 15. Rule catalogue

| ID | Level | Rule | Enforced by |
|---|---|---|---|
| ARCH-001 | MUST | Nothing depends on tooling | `arch` |
| ARCH-002 | MUST | Tier dependency rules | `arch` |
| ARCH-003 | MUST | `test-kit` only as dev-dependency | `arch` |
| ARCH-004 | MUST | Feature slice layout | `arch`, scaffolder |
| ARCH-005 | MUST | `db` module private, items `pub(super)` | `arch`, compiler |
| ARCH-006 | MUST | Entry workflow shape; composition rules | `arch`, ARCH-002 |
| ARCH-007 | MUST | Routers generic over host state | compiler |
| ARCH-008 | MUST | Apps are wiring only | review |
| ARCH-009 | MUST | Handlers registered in `router()` | `arch` |
| SEC-001 | MUST | `OpCtx` drives every workflow/transaction | compiler |
| SEC-002 | MUST | Only auth edges mint `OpCtx` | `arch` |
| SEC-003 | MUST | Permission check first | `arch`, tests, review |
| SEC-004 | MUST | Permission naming; no implicit admin | review, scaffolder |
| SEC-005 | MUST | No token minting in binaries | `arch` |
| SEC-006 | MUST | `Sensitive<T>` for personal data/secrets | review |
| SEC-007 | MUST | Asymmetric token keys outside local | config validation |
| SEC-008 | MUST | Cross-tenant = 404 | RLS, tests |
| DB-001 | MUST | No pools in contexts | `arch` |
| DB-002 | MUST | Role contract + startup posture check; `app_system` limited to `platform.*` | runtime, schema guard |
| DB-003 | MUST | Tenant binding only via `TenantTx`; tenant function pinned | `arch`, schema guard, tests |
| DB-004 | MUST | Tenant-table template / RLS | schema guard |
| DB-005 | MUST | Tenant-leading keys, composite FKs | schema guard |
| DB-006 | MUST | Read-only transactions for queries | review, scaffolder |
| DB-007 | MUST | Bounded reads; keyset pagination | `arch` |
| DB-008 | MUST | Optimistic concurrency by default | tests |
| DB-009 | MUST | SQLSTATE classification | tests |
| DB-010 | MUST | Compile-time checked SQL, `.sqlx` committed | `arch`, CI |
| DB-011 | MUST | Schema per context; migrations touch only their own schema | `arch`, schema guard, CI |
| DB-012 | MUST | Migrators registered; separate migrate step | `arch`, product test |
| DB-013 | MUST | Context SQL only in own schema | `arch` |
| ID-001 | MUST | UUIDv7 identifiers | `arch` |
| ENG-001 | MUST | Engine purity (allowlist, no async) | `arch`, clippy |
| ENG-002 | MUST | Engine determinism | clippy, `arch` |
| ENG-003 | MUST | Integer money, JSON-safe, half-even | `arch`, clippy, types |
| ENG-004 | MUST | Checked arithmetic | `arch`, clippy |
| ENG-005 | MUST | Example + property tests vs oracle/laws | review |
| ENG-006 | MUST | Exhaustive lifecycles | compiler, review |
| API-001 | MUST | problem+json everywhere | middleware, tests |
| API-002 | MUST | Status semantics | tests |
| API-003 | MUST | Problem-aware extractors; strict DTOs | clippy, `arch` |
| API-004 | MUST | Committed OpenAPI contract | tests, CI |
| API-005 | MUST | Same-transaction idempotency | extractor, tests |
| API-006 | MUST | Cursor pagination | types |
| API-007 | MUST | ETag / If-Match | types, tests |
| API-008 | SHOULD | Resource naming and versioning | review |
| MSG-001 | MUST | Outbox in the same transaction | API surface |
| MSG-002 | MUST | Relay semantics | tests |
| MSG-003 | MUST | Versioned, immutable event types | DB check, review |
| MSG-004 | MUST | Idempotent consumers with inbox | runtime, tests |
| MSG-005 | MUST | Broker publish permissions | review/infra |
| MSG-006 | MUST | No personal data in events | review |
| MSG-007 | MUST | Transient vs permanent failures | types, tests |
| MSG-008 | MUST | Trace propagation through events | implementation |
| OBS-001 | MUST | Instrumented workflows (`skip_all`) | `arch` |
| OBS-002 | MUST | Structured logging only | clippy |
| OBS-003 | MUST | Request/tenant/actor on spans | implementation |
| OBS-004 | MUST | W3C trace context + OTLP | implementation |
| OBS-005 | MUST | Log level policy | implementation |
| CFG-001 | MUST | Typed config, env read once | clippy |
| CFG-002 | MUST | Fail fast on invalid config | tests |
| CFG-003 | MUST | Secrets from the secret manager | review |
| OPS-002 | MUST | Liveness vs readiness | tests |
| OPS-003 | MUST | Graceful drain | implementation |
| OPS-004 | MUST | Platform table retention | tests |
| OPS-005 | MUST | Timeouts everywhere | implementation |
| OPS-006 | MUST | Hardened images | CI, manifests |
| LINT-001 | MUST | Workspace lints inherited | `arch`, clippy |
| LINT-002 | MUST | `#[expect(reason)]`, never `#[allow]` | clippy |
| ERR-001 | MUST | Layered errors | review |
| ERR-002 | MUST | No panics in production paths | clippy |
| PERF-001 | SHOULD | Allocator and release profile | `Cargo.toml` |
| PERF-002 | SHOULD | Pool and transaction round trips | implementation |
| PERF-003 | MUST | Set-based reads/writes, no N+1 | review |
| PERF-004 | SHOULD | Shared data behind `Arc` | review |
| PERF-005 | MUST | No `IS NULL OR` in hot queries | review |
| TEST-001 | MUST | Isolation tested as the runtime role | test-kit |
| TEST-002 | MUST | One integration binary per crate | `arch` |
| TEST-003 | MUST | Product HTTP tests | review |
| TEST-004 | MUST | Schema guard per product and context | tests |
| TEST-005 | MUST | Every workflow exercised | `arch` |
| TEST-006 | MUST | ≥ 80 % line coverage | CI |
| TEST-007 | SHOULD | AAA, behavioural names | review |
| GOV-001 | MUST | Rule changes via ADR | CODEOWNERS |
| GOV-002 | MUST | Declared ownership | branch protection |
| SUP-001 | MUST | No known vulnerabilities | CI |
| SUP-002 | MUST | License allowlist | CI |
| SUP-003 | MUST | rustls only; crates.io only | CI |
| SUP-004 | MUST | Pinned toolchain; reviewed updates | CI |
| SCAF-001 | MUST | No unfinished scaffolds | `arch` |
