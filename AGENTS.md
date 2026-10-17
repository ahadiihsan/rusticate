# AGENTS.md

Operating manual for coding agents and engineers in this repository. It applies to every agent (Claude Code, Codex, Cursor, Copilot, Gemini, …) and every human.

- **The standard:** [docs/sop/rust-backend-sop-v2.md](docs/sop/rust-backend-sop-v2.md). Every rule has an ID (e.g. `DB-004`); tool output cites these IDs.
- **The shape:** what `cargo rusticate new domain|feature` scaffolds. Every scaffold already follows every rule; fill in its `SCAFFOLD:` markers instead of writing the structure by hand.
<!-- rusticate:module(example):start -->
- **The example:** the `billing` context ([crates/domains/billing](crates/domains/billing)) is a complete, tested context built that way. Start with [`issue_invoice.rs`](crates/domains/billing/src/issue_invoice.rs). Delete it (`crates/domains/billing*`, its lines in `apps/`) when you no longer need it.
<!-- rusticate:module(example):end -->
- **The enforcement:** `cargo rusticate verify`. If it passes, the change meets the standard; if it fails, the message names the rule.

## Commands

`cargo rusticate <command>` runs this project's own `cargo xtask <command>` (install it once: `cargo install cargo-rusticate --locked`). Where it is not installed, as in CI or a fresh sandbox, run `cargo xtask` with the same arguments.

| Do | Command |
|---|---|
| First time | `cargo rusticate setup` |
| Done? (the gate) | `cargo rusticate verify` (all) · `cargo rusticate verify --fast` (no tests) |
| Architecture only | `cargo rusticate arch` |
| Tests | `cargo rusticate test` · `cargo rusticate test -p <crate>` · `cargo rusticate test -p <crate> -- <name filter>` |
| New context / feature | `cargo rusticate new domain <name>` · `cargo rusticate new feature <ctx> <verb_noun> --kind command\|query` |
| After changing HTTP DTOs or routes | `cargo rusticate openapi` (then review `apps/api/openapi.json`) |
| Run locally | `cargo rusticate run api` · `cargo rusticate token --perm <ctx>.<resource>.<action>` |

<!-- rusticate:module(postgres):start -->
With PostgreSQL:

| Do | Command |
|---|---|
| Start local services (once per session) | `cargo rusticate db up` |
| New migration | `cargo rusticate new migration <ctx> "<what>"` |
| After changing SQL or migrations | `cargo rusticate db migrate` then `cargo rusticate db prepare` |
| Fresh database | `cargo rusticate db reset` |

Builds are offline (`SQLX_OFFLINE=true`): compiling never needs a database. A new or changed `query!` needs `cargo rusticate db prepare` (with `db up`) before it compiles.

<!-- rusticate:module(postgres):end -->
<!-- rusticate:module(messaging):start -->
With messaging: `cargo rusticate new feature <ctx> <verb_noun> --kind event` adds an event consumer, and `cargo rusticate run worker` runs the outbox relay and consumers.

<!-- rusticate:module(messaging):end -->

## Definition of done

1. `cargo rusticate verify` passes, and no `SCAFFOLD:` marker remains.
2. Every new workflow has integration tests: happy path, each failure mode, missing permission, **another tenant cannot see or change it**, and replay or concurrency where relevant.
3. `apps/api/openapi.json` (and each crate's `.sqlx/`, with PostgreSQL) is regenerated if the API (or a query) changed, and you have read the diff.
4. You did not weaken a guardrail (see "Never").

## Shipping a change

`main` accepts no direct or force pushes ([ruleset](.github/rulesets/main.json)). Every change lands as a squash-merged pull request, after CI and CodeQL pass:

```sh
git switch -c <type>/<topic>      # e.g. feat/void-invoice
git push -u origin HEAD
gh pr create --fill               # the PR title becomes the commit: `<type>: <description>`
gh pr merge --auto --squash       # merges itself once the checks are green
```

## Workflow: requirement → code

0. **Plan** (new feature): follow [plan-feature](.agents/skills/plan-feature/SKILL.md) until its tickets (`features/<NNN-slug>/tickets/`) are approved. Then build one ticket per pull request, in dependency order.
1. **Classify.**
   - Which context owns it? A new context needs an ADR.
   - Is it a command (POST), a query (GET), or (with messaging) a reaction to an event?
   - Does it have business rules? They go in `<ctx>-engine`.
   - Can it touch many rows? Then page it or run it as a batch.
<!-- rusticate:module(postgres):start -->
2. **Schema** (if needed): `cargo rusticate new migration`, fill the tenant-table template, then `db migrate`.
<!-- rusticate:module(postgres):end -->
3. **Rules:** pure functions in `<ctx>-engine` with example tests and property tests against an independent oracle or law.
4. **Slice:** `cargo rusticate new feature ...`, then replace every `SCAFFOLD:` in this order: command types → `db.rs` and `db prepare` (with PostgreSQL) → workflow → adapter → tests.
5. **Verify:** `cargo rusticate verify`. Fix the cause; never silence the check.

Skills with the detailed playbooks are in [`.agents/skills/`](.agents/skills):

| Skill | Use when |
|---|---|
| [plan-feature](.agents/skills/plan-feature/SKILL.md) | a new feature: brief → spec → design → tickets under `features/`, before any code |
| [implement-feature](.agents/skills/implement-feature/SKILL.md) | building one ticket of a planned feature, or a small change that needs no plan (the default for code) |
| [write-engine-rules](.agents/skills/write-engine-rules/SKILL.md) | business rules, money, lifecycles, property tests |
| [new-bounded-context](.agents/skills/new-bounded-context/SKILL.md) | a capability no existing context owns |
| [troubleshoot](.agents/skills/troubleshoot/SKILL.md) | a check, build, test or runtime error you don't understand |
| [review-change](.agents/skills/review-change/SKILL.md) | self-review before declaring done, or reviewing someone else |

<!-- rusticate:module(postgres):start -->
With PostgreSQL, [add-migration](.agents/skills/add-migration/SKILL.md) covers any schema change, index or backfill.

<!-- rusticate:module(postgres):end -->
<!-- rusticate:module(messaging):start -->
With messaging, [add-event-consumer](.agents/skills/add-event-consumer/SKILL.md) covers publishing events and reacting to them.

<!-- rusticate:module(messaging):end -->

## Hard rules (the ones most often broken)

- **Feature shape** (ARCH-004/005): `<feature>.rs` holds the workflow, `<feature>/http.rs` (or `handler.rs`) the adapter, and with PostgreSQL `<feature>/db.rs` the private SQL. No business logic in adapters or `db.rs`; no SQL outside `db.rs`.
- **Entry workflows** (SEC-003, OBS-001): take `ctx: &OpCtx` first, call `ctx.require(PERM)?` first, and carry `#[tracing::instrument(skip_all, ...)]`.
<!-- rusticate:module(postgres):start -->
- **Tenant access** (DB-001/003, DB-013): open transactions only with `db.begin(ctx)` or `db.begin_read_only(ctx)`. Queries run against `&mut TenantTx` and include `tenant_id = $1`. A context queries only its own schema.
- **SQL** (DB-010, DB-007, PERF-003, PERF-005): compile-time checked `sqlx::query!` family only. Every `fetch_all` is bounded (`LIMIT` or `= ANY`). Write N rows with `UNNEST`, never N statements. No `($x IS NULL OR …)` filters.
- **Writes** (API-005, API-007, DB-008): external-effect mutations take `IdempotencyKey`. Updates are version-guarded (`If-Match`).
<!-- rusticate:module(postgres):end -->
<!-- rusticate:module(messaging):start -->
- **Events** (MSG-001): events go through `tx.emit(..)` in the same transaction, never straight to the broker.
<!-- rusticate:module(messaging):end -->
- **Money and engines** (ENG-001..004): money is `i64` minor units with a currency; never `f32`/`f64`. Engines have no async and no I/O, and take time and ids as inputs.
- **HTTP** (API-001/003): use `api_kit::{Json, Path, Query}`. Workflows return the context's error enum, never `ApiError`; the HTTP mapping lives in `error.rs`. `*Request` DTOs use `deny_unknown_fields`.
- **Safety** (ERR-002, SEC-006): no `unwrap`/`expect`/panics/indexing outside tests. Personal data is wrapped in `Sensitive<T>`.
- **Identifiers** (ID-001): UUIDv7 (`Uuid::now_v7()`).
<!-- rusticate:module(postgres):start -->
- **Migrations** (DB-011): new file each time, schema-qualified, tenant template, `SET LOCAL lock_timeout`. Merged migrations are never edited.
<!-- rusticate:module(postgres):end -->

## Never

- Add `#[allow(..)]`. Use `#[expect(lint, reason = "...")]` only when the code is right and the lint is wrong.
- Edit `architecture.toml`, `clippy.toml`, `deny.toml`, `rust-toolchain.toml`, CI, or another crate's lints to make your change pass. Those changes need an ADR and platform review (GOV-001).
<!-- rusticate:module(postgres):start -->
- Disable or weaken RLS, grant `TRUNCATE`/`BYPASSRLS`, use `SET ROLE`/`set_config`, or connect with a pool from a context.
- Edit a merged migration.
<!-- rusticate:module(postgres):end -->
- Delete or rewrite tests to make them pass.
- Construct an `OpCtx` outside `iam`, `messaging` or tests (`op_context::mint`).
- Put secrets in code or committed files. Local defaults live only in `xtask/src/dev_env.rs`.

## Where things live

```text
apps/api, apps/worker          composition roots, wiring only (the worker comes with messaging)
crates/foundation/*            platform: op-context, api-kit, server-kit, telemetry (+ datastore, idempotency, test-kit with PostgreSQL; messaging)
crates/domains/<ctx>           bounded context: router(), permissions, features (+ MIGRATOR with PostgreSQL, events with messaging)
crates/domains/<ctx>-engine    pure rules of the context
architecture.toml              tiers, allowlists, capabilities (read by `cargo rusticate arch`)
xtask/                         enforcement and scaffolding
docs/                          SOP, ADRs, configuration, operations
```

<!-- rusticate:template-only:start -->
`cargo-rusticate/` is the project generator (`cargo rusticate new`): a standalone crate outside the workspace, never copied into generated projects (nor is `.github/workflows/release.yml`, which publishes it). Anything you add to the template ships to every new project, so the generator renames `rusticate`, `Rusticate`, the repository URL and `@ahadiihsan`, and refuses any other spelling of them. Content that only makes sense in this repository goes inside a template-only block, delimited by HTML comment lines like the ones around this paragraph.

Optional modules (`postgres`, `messaging`, `example`) are declared in [`template.toml`](template.toml) with the paths each owns ([ADR 0002](docs/adr/0002-composable-template-modules.md)). A line that belongs to a module in a shared file goes between `module(<name>)` start and end marker comments, in the file's own comment syntax; look at `apps/api/src/main.rs` or this file for the form. `cargo rusticate ci` generates a project and runs its `verify --fast`. Details: [cargo-rusticate/README.md](cargo-rusticate/README.md).

<!-- rusticate:template-only:end -->
## When stuck

- An `arch` violation names a rule ID: read that rule in the SOP. The fix is almost always to move code to where the rule wants it, not to work around the check.
- Common failures and their fixes: [troubleshoot](.agents/skills/troubleshoot/SKILL.md).
- If the right change needs a rule exception, stop and say so in the PR or to the user, with the rule ID and the reason. Don't hide it.
