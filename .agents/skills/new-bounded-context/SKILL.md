---
name: new-bounded-context
description: Create a new bounded context (domain crate + engine crate + schema) when a capability belongs to no existing context. Use only after confirming no existing context owns the language and data; includes the decision criteria and the ADR requirement.
---

# Create a bounded context

## Decide first: is it really a new context?

A context owns a coherent **language** (its terms mean one thing inside it), its **data** (schema), and its **invariants**. Create one when the capability:

- has its own vocabulary that would be ambiguous inside an existing context ("account" in billing and in IAM are different things);
- owns data no other context should write;
- changes for different reasons, or is owned by a different team.

Do **not** create one for a feature of an existing context, or to share code. Shared technical code goes in `crates/foundation` (platform review).

A new context changes the context map, so it **needs an ADR** in [`docs/adr/`](../../../docs/adr) (GOV-001). Write the ADR first.

## Create

```sh
cargo xtask new domain shipping
cargo xtask db migrate
```

This generates:

- `crates/domains/shipping/`
  - `Cargo.toml`, and `sqlx.toml` (schema `shipping`, its own migration table)
  - `migrations/<ts>_shipping_init.sql`
  - `src/lib.rs` (`router()`, `MIGRATOR`, `xtask:` markers), `error.rs`, `permissions.rs`, `events.rs`
  - `tests/integration/` (one binary, with a schema-guard test)
- `crates/domains/shipping-engine/` (allowlisted deps, a stricter `clippy.toml`, lint attributes)

It also wires the context: workspace dependencies, the API's dependencies, `.merge(shipping::router())` and `MIGRATORS`.

Then:

1. Replace the `SCAFFOLD:` markers in `lib.rs` (the context's purpose) and in the engine.
2. Add an OpenAPI tag in [`apps/api/src/openapi.rs`](../../../apps/api/src/openapi.rs) (`tags((name = "shipping", description = "..."))`).
3. Add CODEOWNERS entries for both crates.
4. Add features with [implement-feature](../implement-feature/SKILL.md).

## Relationships with other contexts

| Need | Do |
|---|---|
| React to something another context did | consume its events ([add-event-consumer](../add-event-consumer/SKILL.md)) |
| Another context must react to you | publish events |
| Synchronous call into another context | avoid. If unavoidable, the callee must be a *shared* context (`[domains].shared` in `architecture.toml`), approved by ADR. `iam` is the only one today. |
| Read another context's tables | never (DB-013); `cargo xtask arch` rejects SQL that references another context's schema |

A context must be mountable into any product binary: its router is generic over the host state, and its migrations live with it.
