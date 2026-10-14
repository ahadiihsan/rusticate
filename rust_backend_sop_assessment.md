# Assessment: Rust Backend Architecture & Engineering SOP

**Subject:** [rust_backend_architecture_engineering_sop.md](rust_backend_architecture_engineering_sop.md) (1,539 lines)
**Date:** 2026-10-14
**Scope:** Practicality, best practice, performance, rigidity, and how well the rules are enforced for engineers and AI agents.

Line references point to the SOP file, which this assessment leaves unchanged.

---

## Verdict

**The architecture is sound. As a document you can enforce, it is not ready.** The core choices are right for a multi-tenant Rust backend: a crate dependency graph, pure engine crates with no I/O, transaction-local Postgres row-level security (RLS), a transactional outbox, UUIDv7 keys, RFC 9457 errors, and a decimal `Money` type. Keep all of them.

There are three problems:

1. **Several code samples are buggy, and two security bugs break tenant isolation.** The SOP says agents "MUST follow this exact structural pattern", so they will copy the bugs exactly.
2. **Most "enforcement" is just prose.** The document says some failures are "physically impossible" when nothing checks them.
3. **Key production pieces are missing:** the outbox relay, the idempotency implementation, timeouts, migration ownership, and the shutdown code.

---

## 1. Bugs in the code samples (fix before anyone copies them)

| # | Where | Bug | Impact / fix |
|---|---|---|---|
| 1 | [L455-459](rust_backend_architecture_engineering_sop.md#L455-L459) | The policy is `AS RESTRICTIVE` and there is no permissive policy. Postgres denies everything unless at least one *permissive* policy grants access. | **Every query returns 0 rows and every INSERT fails the policy check.** Drop `AS RESTRICTIVE`, or add a permissive policy alongside it. |
| 2 | §3.1 and the test at [L1450](rust_backend_architecture_engineering_sop.md#L1450) | It never says the app must connect as a role that is **not the table owner, not a superuser, and has no `BYPASSRLS`**. `FORCE ROW LEVEL SECURITY` only covers the owner; superusers skip RLS entirely. `#[sqlx::test]` usually connects as `postgres`, a superuser. | RLS is silently off in dev and CI. The canonical isolation test fails against a standard docker Postgres. Add an `app_rw` role and use `SET ROLE` in the test harness. |
| 3 | Table template [L434-449](rust_backend_architecture_engineering_sop.md#L434-L449) | Foreign-key and unique checks **bypass RLS** (documented Postgres behaviour). | Tenant A can insert a row pointing at tenant B's `account_id`, and unique-constraint errors reveal that values exist in other tenants. Require composite FKs `(tenant_id, x) REFERENCES t(tenant_id, id)` and uniques that start with `tenant_id`. |
| 4 | Streamer [L575-581](rust_backend_architecture_engineering_sop.md#L575-L581) | It calls `set_config(..., false)`, which is **session-level**, on a pooled connection. It also takes a raw `&PgPool`, which breaks the SOP's own rule. | The connection goes back to the pool still carrying tenant A's context. Any later code on that connection that doesn't open a tenant transaction (relay, admin, health) runs as tenant A. |
| 5 | [L205](rust_backend_architecture_engineering_sop.md#L205), [L458](rust_backend_architecture_engineering_sop.md#L458) | After a transaction-local GUC has been set once on a session, `current_setting('app.current_tenant', true)` returns `''` rather than NULL, and `''::uuid` raises an error. | Unscoped queries on a reused connection fail with a confusing error instead of returning empty. Add one `STABLE` SQL function, `app.tenant_id()` = `NULLIF(current_setting('app.current_tenant', true), '')::uuid`, and use it in every policy and default. |
| 6 | [L289](rust_backend_architecture_engineering_sop.md#L289) | `OpCtx::new_authenticated` is `pub`. | Any domain crate can forge an `OpCtx`, so it is not "unforgeable". Put the constructor behind a feature such as `construct`, and have a check confirm only `shared-iam`, `apps/*` and tests enable it. |
| 7 | [L917](rust_backend_architecture_engineering_sop.md#L917) | `trace_id = Uuid::now_v7()` is generated fresh inside `IntoResponse`. | The trace_id the support team sees **matches nothing** in your traces. Use the trace ID from the current OpenTelemetry span (a hex string, not a UUID). `instance` is required by [L838](rust_backend_architecture_engineering_sop.md#L838) but missing from the struct. |
| 8 | [L179](rust_backend_architecture_engineering_sop.md#L179) vs. [L524](rust_backend_architecture_engineering_sop.md#L524), [L666](rust_backend_architecture_engineering_sop.md#L666), [L1400](rust_backend_architecture_engineering_sop.md#L1400) | The rule says queries are "compile-time checked", but **every example uses runtime `sqlx::query(...)`**. | Agents will copy the unchecked form, so `cargo sqlx prepare` checks nothing. The examples must use `query!` / `query_as!`. |
| 9 | [L1190](rust_backend_architecture_engineering_sop.md#L1190) | `unit_price: Decimal` is deserialized from a JSON *number*. serde_json parses it as an `f64` first. | Floating-point precision loss at the HTTP boundary, which the SOP says can't happen. Use `#[serde(with = "rust_decimal::serde::str")]`, or integer minor units in the API. |
| 10 | [L744](rust_backend_architecture_engineering_sop.md#L744), [L751](rust_backend_architecture_engineering_sop.md#L751) | `Money` derives `Deserialize`, which skips `new()`. `new()` also silently rounds `10.005 USD` instead of rejecting it. | The "parse, don't validate" rule is broken in the type meant to show it. Use `#[serde(try_from = ...)]` and reject excess precision at the boundary. |
| 11 | [L1450](rust_backend_architecture_engineering_sop.md#L1450) | `migrations = "../../migrations"` from `crates/domains/domain-alpha` resolves to `crates/migrations`. | Wrong path; it needs `../../../`. The workspace tree also has no `migrations/` folder at all. |
| 12 | [L1260](rust_backend_architecture_engineering_sop.md#L1260) vs. [L187](rust_backend_architecture_engineering_sop.md#L187) | Handlers take `State<DataStore>`, but routers are typed `OpenApiRouter<AppState>`, and `AppState` lives in `apps/`, which domain crates cannot depend on. | Doesn't compile as designed. Make routers generic: `router<S>() where DataStore: FromRef<S>`. |
| 13 | [L1181](rust_backend_architecture_engineering_sop.md#L1181) | The reference endpoint takes `tax_rate_bps` and `unit_price` from the client. | It teaches agents to trust prices and tax rates sent by the client. Those must be loaded on the server. |
| 14 | [L1146-1150](rust_backend_architecture_engineering_sop.md#L1146-L1150) | Invariant 3 recomputes the result exactly the way the implementation does. | It's a tautology and proves nothing. Test real invariants instead: the sum of line nets equals the net subtotal, results are the same regardless of line order, and the per-line rounding error stays bounded. |

**Smaller issues**

- `#[instrument(skip(db, cmd))]` ([L1315](rust_backend_architecture_engineering_sop.md#L1315)) still records `opctx` through `Debug`, including the permission set. Use `skip_all` and list fields explicitly.
- `resolver = "2"` ([L69](rust_backend_architecture_engineering_sop.md#L69)) should be `"3"` for edition 2024.
- `sqlx`'s combined `runtime-tokio-rustls` feature is legacy; split it into a runtime feature and a TLS feature.
- `Money` isn't in scope in the test module at [L1476](rust_backend_architecture_engineering_sop.md#L1476).
- `clippy.toml` is missing `allow-indexing-slicing-in-tests` and `allow-print-in-tests`, which you need because `indexing_slicing` and `print_*` are set to deny.

---

## 2. Enforcement: what the document claims vs. what is actually checked

| Rule | What the SOP claims | What actually happens | Mechanism to add |
|---|---|---|---|
| Engine crates have no tokio or sqlx | "Physically absent" | Nothing checks `Cargo.toml`; anyone can add the dependency. | `cargo xtask arch-check` using `cargo_metadata`, with the dependency matrix from §2.1 written as data. Fails CI. |
| Engines are deterministic | Stated | `Utc::now()`, `Uuid::now_v7()` and `rand` all compile fine. The SOP's own proptest calls `now_v7()`. | A per-engine `clippy.toml` with `disallowed-methods`/`disallowed-types` covering clocks, RNG, `std::fs` and `std::net`. Inject `now` as an input. |
| No floating-point money | "Physically impossible" ([L190](rust_backend_architecture_engineering_sop.md#L190)) | `float_arithmetic` only flags float operators. `as f64`, `Decimal::from_f64` and JSON numbers all get through. | Disallow `from_f64`/`to_f64` in clippy config. Use string decimals in DTOs. |
| Integer overflow | Not mentioned | `i64` minor-unit arithmetic can wrap silently in release builds. | `arithmetic_side_effects = "deny"` in engine crates, plus `overflow-checks = true` in `[profile.release]`. |
| No N+1 queries | "Physically impossible" ([L190](rust_backend_architecture_engineering_sop.md#L190)) | Engines can't query the database, but orchestrators can loop over `db::*` calls. | Rule: Hop 3 functions taking collections must be set-based (`= ANY($1)`). Check it in review or with query-count assertions in tests. |
| Every table has RLS | "MUST" | Nothing verifies it. | **One `#[sqlx::test]`** that queries `pg_class`/`pg_policies` and fails if any table with a `tenant_id` column lacks `relrowsecurity`, `relforcerowsecurity`, or a permissive policy. Cheap, and covers every table. |
| Lint exceptions | Not mentioned | `#[allow(clippy::unwrap_used)]` quietly gets around the zero-panic rule. | Deny `clippy::allow_attributes` and `clippy::allow_attributes_without_reason`, which forces `#[expect(lint, reason = "...")]`. |
| Migration safety | Not mentioned | — | `squawk` in CI, plus an expand/contract policy for zero-downtime changes. |
| Canonical code is correct | "Follow exactly" | The snippets have never been compiled; several are broken (§1). | **Make the reference slice a real crate in the workspace that CI builds and tests**, and have the SOP link to it instead of embedding copies. |
| Feature structure | Prose | Agents drift from prose. | `cargo xtask new-feature <context> <verb_noun>` generates the 3-hop skeleton, tests included. A generator enforces structure far better than a 1,500-line document. |

With `pedantic = warn` plus `-D warnings`, all of pedantic is effectively deny, and the canonical snippets will probably trip lints such as `needless_pass_by_value` on `cmd`. Say this explicitly, or pick a curated list of lints.

---

## 3. Performance

- **Replace the long-running streaming cursor with keyset pagination.** `.fetch()` is not a server-side cursor. It is one query that holds a pooled connection and a snapshot for the whole dispatch, possibly hours of NATS acks for 2.5M rows. That pins the transaction-ID horizon (so vacuum can't clean up and tables bloat) and can't resume after a crash. Instead, run `WHERE id > $last ORDER BY id LIMIT 1000` in short `TenantTx` transactions and save the last ID as a checkpoint. It resumes, has the same memory profile, and needs no raw pool.
- **Outbox table:** updating `status` to `'PENDING'` and then to published ([L529](rust_backend_architecture_engineering_sop.md#L529)) rewrites rows and causes bloat. Delete on publish, or partition by time and drop old partitions. Relay with `FOR UPDATE SKIP LOCKED`, and set `Nats-Msg-Id = event.id` so JetStream deduplicates. Bind `sqlx::types::Json(payload)` directly instead of `to_value` ([L522](rust_backend_architecture_engineering_sop.md#L522)) to avoid converting the payload twice.
- **`OpCtx` is cloned on every request**, along with its `HashSet<Permission>` containing `Custom(String)` values ([L263](rust_backend_architecture_engineering_sop.md#L263), [L284](rust_backend_architecture_engineering_sop.md#L284)). Use `Arc<[Permission]>` or a bitset, with `Permission(&'static str)` constants defined per domain. This also fixes a design problem: foundation code shouldn't define domain permissions like `ResourceWrite`.
- **DB error mapping:** `40001` (serialization failure), `40P01` (deadlock) and `23505` (unique violation) currently become 500s. Map them in a `From<DataStoreError> for ApiError` to a retryable 503 or a 409. That also removes the six `.map_err(|e| ApiError::Internal(e.into()))` calls.
- **Missing runtime settings:**
  - Per-role `statement_timeout` and `idle_in_transaction_session_timeout`.
  - Pool `acquire_timeout` and pool sizing.
  - A Tower `TimeoutLayer` and `DefaultBodyLimit`.
  - mimalloc or jemalloc as the allocator.
  - A non-blocking JSON log writer.
  - `debug = "line-tables-only"` in release for readable backtraces.
  - Keep `panic = "unwind"`, because the `catch-panic` layer depends on it.
- **Build speed at scale:** cargo-hakari (a workspace-hack crate), `SQLX_OFFLINE=true` in CI, and a fast linker.

---

## 4. Practicality and design gaps

- **Orchestrators own the transaction, so two workflows can't be combined atomically.** Use a standard split: `issue_invoice_in_tx(tx: &mut TenantTx, …)` plus a thin `issue_invoice(opctx, db, cmd)` wrapper. Also state the rule for one domain crate depending on another: only `shared-*` directly; everything else communicates through events.
- **Orchestrators return `ApiError`** ([L1329](rust_backend_architecture_engineering_sop.md#L1329)), which ties HTTP types to workflows that workers also call. The `error.rs` the SOP itself prescribes ([L167](rust_backend_architecture_engineering_sop.md#L167)) goes unused in the reference.
- **Idempotency** ([L627](rust_backend_architecture_engineering_sop.md#L627)) has no code, and it hides a hard problem. Tower middleware can't share the business transaction, so a crash leaves the key `IN_PROGRESS` forever. Either finalize the key inside the same `TenantTx` (pass it to Hop 2), or add a lease column with an expiry. Make it a typed extractor (`Idempotent<Json<T>>`) so each route has to opt in or out explicitly.
- **Missing:**
  - The outbox relay, which needs a cross-tenant `BYPASSRLS` role and so contradicts the RLS rule unless designed for.
  - Consumer deduplication (an inbox table).
  - Who owns migrations when three products share one `migrations/` folder.
  - Config validated at startup.
  - Rate limiting.
  - Shutdown details: readiness returning 503 isn't enough. You also need a `preStop` sleep (endpoint removal propagates slowly) and `terminationGracePeriodSeconds` set above the drain timeout.
- **The SOP only covers writes.** Add a read/query pattern: no engine, a read-only transaction, optionally a replica.
- **A per-feature cross-tenant test is copy-paste.** Use one generic RLS harness that runs over all tables, plus the catalog check from §2. Per-feature tests can then focus on behaviour.
- **Storage format for money is undecided:** the SOP allows "BIGINT minor units OR NUMERIC(20,4)" ([L702](rust_backend_architecture_engineering_sop.md#L702)), and the examples use both. Choose one.

---

## 5. Making it usable for agents

1,500 lines is too much context to give an agent on every task, and long prose weakens rule-following. Split it up:

- **`AGENTS.md`, at most about 150 lines.** Rules get IDs (e.g. `DB-003`), RFC 2119 keywords, and each one names its enforcer (compiler, clippy, xtask, CI test, or review). Include the exact commands to run.
- **Per-task skills:** `new-feature`, `migration`, `batch-job`, `outbox-consumer`, each pointing to the compiled reference crate.
- **Hooks:** run `cargo clippy -p <crate>` and `xtask arch-check` after edits, so agents get mechanical feedback rather than relying on reading the rules.
- **Separate the argument from the instructions.** The "Real-World Context" sections justify the design for humans. Agents need rules, and those sections dilute them.

---

## What to do, in order

1. **Fix the security bugs (§1 #1–6).** The RLS policy, the app role, composite FKs, the streamer, the `NULLIF` function, and the `OpCtx` constructor.
2. Turn the reference slice into a crate CI compiles and tests, using `query!` macros. Remove code copies from the SOP.
3. Add `xtask arch-check`, the RLS catalog test, per-engine `clippy.toml` files, the `#[expect(reason)]` requirement, `arithmetic_side_effects`, and squawk.
4. Specify the outbox relay, the idempotency storage design, keyset batching, and the timeouts.
5. Restructure into `AGENTS.md`, skills, the scaffolding generator, and hooks.
