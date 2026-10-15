# SOP v1 → v2: what changed and why

[SOP v1](rust-backend-sop-v1.md) is kept unchanged. This log maps each v1 problem to its v2 resolution and where it is enforced. The detailed review of v1 is in the git history: [rust_backend_sop_assessment.md](https://github.com/ahadiihsan/rusticate/blob/af3cac3b0968a2b0c6ff767461838986a65032a1/rust_backend_sop_assessment.md) (its line references point into v1).

## Kept from v1

The direction was right and is unchanged:
- crate-tier dependency graph;
- sans-I/O engines;
- capability-style `OpCtx`;
- transaction-local RLS binding;
- transactional outbox;
- UUIDv7 keys;
- RFC 9457 errors;
- integer-safe money;
- `utoipa` contracts;
- feature folders named in the ubiquitous language.

## Security defects fixed

| # | v1 | Effect | v2 | Enforced by |
|---|---|---|---|---|
| 1 | Only a `RESTRICTIVE` RLS policy | Postgres denies every row: the app cannot read or insert anything | One **permissive** `FOR ALL` policy with a canonical predicate; any other permissive policy is rejected | schema guard DB-004b/c |
| 2 | No role contract; tests ran as a superuser | RLS silently off in dev/CI; the isolation test cannot fail | `app_owner` / `app_rw` / `app_system`; stores check the role at startup; tests `SET ROLE` to the real runtime roles | posture check, `TestDb`, DB-002a/b |
| 3 | Plain foreign keys and global unique indexes | FK checks bypass RLS: cross-tenant references and existence leaks | Composite `(tenant_id, …)` keys, FKs and uniques | schema guard DB-005a–c |
| 4 | Streamer used session-level `set_config(.., false)` on a pooled connection | A later borrower runs as the previous tenant | Only transaction-local binding inside `TenantTx`; `set_config` banned elsewhere | `arch` (`session_settings`), test |
| 5 | `current_setting(..)::uuid` | `''` after a transaction-local setting ends → cast error | `platform.current_tenant_id()` with `NULLIF` | migration, test |
| 6 | `OpCtx::new_authenticated` was `pub` | Any crate could forge identity | `mint` capability restricted to auth edges | `arch` (`mint`) |
| 7 | `TRUNCATE` not considered | `TRUNCATE` ignores RLS | Never granted | schema guard DB-002a |
| 8 | Views and `SECURITY DEFINER` not considered | Both can bypass RLS | `security_invoker` views; reviewed definer functions | schema guard DB-004f/g |
| 9 | Transitive `rsa` crate (RUSTSEC-2023-0071) via JWT backend | Marvin timing side channel | `jsonwebtoken` on `aws_lc_rs`; `cargo deny` fails on advisories | `deny.toml`, CI |

## Correctness defects fixed

| v1 | v2 |
|---|---|
| `trace_id` freshly generated in `IntoResponse`, so it matched nothing | the W3C trace id of the request span, plus `request_id` and `instance` |
| Examples used runtime `sqlx::query(..)` while claiming compile-time checking | `query!` family only; runtime SQL is a restricted capability; `.sqlx` committed; offline builds |
| JSON-number `Decimal` request fields (parsed as `f64`) | integer minor units in the API; `i64` money capped at 2⁵³−1 |
| `Money` derived `Deserialize` (bypassing validation) and silently rounded input | validated constructors; rounding once, half-to-even, in one place |
| Tautological property test (restated the implementation) | properties against an independent oracle (`rust_decimal`) and algebraic laws |
| Client-supplied tax rate | server-side tenant settings |
| Workflows returned `ApiError` | context error enum; one HTTP mapping; one retry mapping for consumers |
| `migrations = "../../migrations"` resolved to the wrong directory; one global migrations folder | each context owns its schema and migration history (`sqlx.toml`), mounted per product |
| Routers typed on the app's `AppState` (domain crates could not compile against it) | routers generic over host state (`DataStore: FromRef<S>`) |
| `#[instrument(skip(db, cmd))]` still recorded `OpCtx` | `skip_all` with explicit fields, enforced |
| Idempotency layer as separate middleware: a crash left keys stuck "in progress" | claim + complete in the business transaction; concurrent duplicates wait and replay |

## Performance changes

| v1 | v2 |
|---|---|
| One long streaming query for million-row jobs: pins a connection and the xmin horizon (vacuum stalls), and cannot resume | keyset pages in short transactions |
| Outbox `status` column updated in place (bloat) | relay deletes after publish; global id order; single leader by advisory lock; pipelined acks |
| `HashSet<Permission>` with `String`s cloned per request | `Arc`'d context, sorted slice, binary search |
| Liveness ping on every pool acquire; separate `SET` round trips | `test_before_acquire(false)`; timeouts in the startup packet; `BEGIN READ ONLY` in one statement |
| N+1-prone examples | set-based `UNNEST` inserts, `= ANY` reads, enforced bounds on `fetch_all` |
| `($x IS NULL OR …)` style filters | one query per filter shape with sentinel bounds (generic-plan safe) |
| No allocator or release profile guidance | mimalloc, thin LTO, `codegen-units = 1`, overflow checks kept |

## Enforcement: from prose to machines

| v1 claim | v2 mechanism |
|---|---|
| "Physically absent from Cargo.toml" | `architecture.toml` + `cargo rusticate arch` dependency rules (allowlist for engines) |
| "Physically impossible" float bugs | `arch` bans `f32`/`f64` in engines; `float_arithmetic` denied; integer money types |
| Determinism (implied) | engine `clippy.toml` bans clocks, id generation and hash iteration; `arch` keeps it in sync with the root config |
| Overflow (not mentioned) | `arithmetic_side_effects` + `as_conversions` denied in engines; release overflow checks |
| RLS on every table ("MUST") | catalog schema guard over the *migrated* database, in every context's and product's tests |
| `#[allow]` escapes (not mentioned) | `allow_attributes` denied; `#[expect(reason)]` only |
| Code samples in a document | the `billing` reference compiles and is tested; generators emit conforming slices |
| Feature layout by convention | `arch` checks slice shape, private SQL, instrumentation, `OpCtx` first parameter, route registration, workflows covered by tests |
| Supply chain (partial) | `cargo deny` (advisories, licenses, bans, sources), Dependabot, pinned toolchain |
| Migration safety (not mentioned) | squawk in CI, immutability check, timeouts in every migration |
| Agent instructions in one 1,500-line file | short `AGENTS.md` + task skills + hooks; every rule has an ID, and tool output cites it |

## Added in v2 (absent from v1)

- Outbox relay, consumer runtime with inbox dedup, retries vs dead-lettering, consumer naming, trace propagation through events.
- HTTP preconditions (`ETag`/`If-Match`/`If-None-Match`, 412/428).
- Load shedding, request deadlines, body limits, panic-to-problem, security headers, CORS policy.
- Health probes, a shutdown sequence that works without a shell in distroless images, Kubernetes manifests, a migrate Job.
- Typed configuration that fails fast, with environment-specific validation (no HS256 or docs UI in production).
- Cursor pagination and the full status-code semantics table.
- Coverage floor, product-level HTTP tests, end-to-end pipeline tests against real JetStream.
- Governance: ADRs, CODEOWNERS, PR template, escape-hatch table.
