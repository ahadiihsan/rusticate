# 0001. Adopt the v2 engineering standard and enforce it with tooling

- **Status:** Accepted
- **Date:** 2026-10-14
- **Deciders:** Platform team
- **Rules affected:** all (see the [rule catalogue](../sop/rust-backend-sop-v2.md#15-rule-catalogue))

## Context

SOP v1 set the right direction, but:

- several of its canonical code listings were wrong, two of them in ways that disable tenant isolation;
- most of its rules relied on reviewers remembering them;
- at 1,500 lines it was too long for coding agents to apply reliably.

The full analysis is in [v1-to-v2.md](../sop/v1-to-v2.md).

## Decision

1. The standard is [SOP v2](../sop/rust-backend-sop-v2.md). Every rule has an ID, a level and an enforcer.
2. The `billing` context is the compiling, tested reference implementation. Documents link to it instead of embedding code.
3. Machine-checkable rules are enforced by the compiler, clippy, `cargo rusticate arch` (from `architecture.toml`), the catalog schema guard, `cargo-deny`, squawk and the test suite. `cargo rusticate verify` is the definition of done locally and `cargo rusticate ci` in CI.
4. Bounded contexts own their PostgreSQL schema and migration history (`sqlx.toml` per crate).
5. Idempotency keys are claimed and completed in the business transaction.
6. Money is `i64` minor units with an explicit currency, within the JSON-safe range.
7. Agents get `AGENTS.md`, task skills in `.agents/skills`, and hooks that run the fast gate.

## Alternatives considered

| Option | Why not |
|---|---|
| Fix v1's listings in place | Leaves enforcement to memory; listings drift from code again |
| `cargo-deny` `[bans]` wrappers for the crate graph | Cannot express "a domain may depend on its own engine only"; no source-level capabilities |
| Global `migrations/` directory | Couples every context to every product database; one shared history |
| Idempotency as Tower middleware with a lease | A crash leaves keys "in progress" until the lease expires; needs a second transaction |
| `rust_decimal` for stored money | Larger rows and slower arithmetic; JSON numbers parsed as `f64` lose precision unless sent as strings |

## Consequences

- New contributors and agents get fast, specific feedback with rule IDs instead of review comments.
- Changing a rule is a code change, reviewed by the platform owners.
- The platform team maintains `xtask` and the schema guard, and keeps their detection tested. The source-rule unit tests are in `xtask/src/arch/source.rs`. The guard's negative test (`schema_guard_detects_isolation_mistakes`) creates deliberately broken tables and asserts each violation is reported. When adopting this standard, the checker was also mutation-tested by hand: 12 kinds of violations were planted in the reference context, and every one was reported with its rule ID.
- Two small manual reviews remain: SEC-006 (personal data typing) and MSG-005/006 (broker permissions, event payloads).
