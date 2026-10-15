---
name: implement-feature
description: Turn a PRD, TRD, ticket or feature request into a finished feature slice in this Rust workspace (workflow, HTTP or event adapter, SQL, engine rules, tests, contract). Use for any new endpoint, use case or behaviour change in a bounded context.
---

# Implement a feature

The goal is a change that passes `cargo rusticate verify`. The shape is what `cargo rusticate new feature` scaffolds: it already follows every rule, so you fill in its `SCAFFOLD:` markers rather than invent structure. Rules are cited by ID; their reasons are in the [SOP](../../../docs/sop/rust-backend-sop-v2.md).
<!-- rusticate:module(example):start -->

For a finished example of every step, read the [`billing`](../../../crates/domains/billing) context, starting with [`issue_invoice.rs`](../../../crates/domains/billing/src/issue_invoice.rs).
<!-- rusticate:module(example):end -->

## 1. Classify before writing code

Answer these in your notes or PR description:

| Question | Decides |
|---|---|
| Which bounded context owns this, in its own words? | the crate; a *new* context needs [new-bounded-context](../new-bounded-context/SKILL.md) |
| Does it change state (command) or read it (query)? | `--kind command\|query` |
| Name it `<verb>_<noun>` in the domain language (`issue_invoice`, not `create_invoice_record`) | module, permission `<ctx>.<noun>.<verb>`, route |
| Which business rules or calculations? | engine functions ([write-engine-rules](../write-engine-rules/SKILL.md)) |
| Which data does the server decide (prices, rates, limits), never the client? | what the workflow loads instead of accepting |
| Does it handle personal data? | `Sensitive<T>`, no personal data in logs (SEC-006) |

<!-- rusticate:module(postgres):start -->
With PostgreSQL, also:

| Question | Decides |
|---|---|
| External effect (money, stock, messages)? | `Idempotency-Key` (API-005) |
| Updates an existing aggregate? | `version` + `If-Match` (API-007, DB-008) |
| Can it touch unbounded rows? | keyset pagination or a batch job (DB-007) |

<!-- rusticate:module(postgres):end -->
<!-- rusticate:module(messaging):start -->
With messaging: should other contexts know it happened? Then publish an event, or if this feature *reacts* to one, use `--kind event` ([add-event-consumer](../add-event-consumer/SKILL.md)). Events carry no personal data (MSG-006).

<!-- rusticate:module(messaging):end -->
<!-- rusticate:module(postgres):start -->
## 2. Schema (only if tables change)

Follow [add-migration](../add-migration/SKILL.md): `cargo rusticate new migration <ctx> "<what>"`, fill the tenant template, then `cargo rusticate db migrate`.

<!-- rusticate:module(postgres):end -->
## 3. Business rules first

Write pure functions in `<ctx>-engine`, with error variants and with example and property tests. Run `cargo rusticate test -p <ctx>-engine`. This is the cheapest place to get the logic right.

## 4. Scaffold the slice

```sh
cargo rusticate new feature <ctx> <verb_noun> --kind command   # or query
```

This creates `<feature>.rs`, `<feature>/http.rs` and `tests/integration/<feature>.rs` (with PostgreSQL also `<feature>/db.rs`). It adds the permission constant, mounts the route, and runs `cargo fmt`. `cargo rusticate arch` lists every `SCAFFOLD:` marker; replace them **in this order**:

1. **Command / view types** in `<feature>.rs`. Fields are value objects (`CustomerEmail`, `Quantity`, `InvoiceId`), never raw strings or numbers. Add new value objects to `types.rs` with a `parse` constructor.
<!-- rusticate:module(postgres):start -->
2. **SQL** in `<feature>/db.rs`. Use the `query!` family against `&mut TenantTx`, with `WHERE tenant_id = $1` on every statement. Return rows; convert to domain types in the workflow. Then run `cargo rusticate db prepare`.
<!-- rusticate:module(postgres):end -->
3. **Workflow** in `<feature>.rs`, in the order the scaffold lays out: `ctx.require(permissions::X)?` first, then load server-side facts, call the engine, and return the context error enum (never `ApiError`).
<!-- rusticate:module(postgres):start -->
   With PostgreSQL the full order is:
   1. `ctx.require(permissions::X)?`;
   2. for commands, `Fingerprint::of` and `db.begin(ctx)`; for queries, `db.begin_read_only(ctx)`;
   3. `idempotency::claim` (replay returns early);
   4. load server-side facts;
   5. call the engine;
   6. persist set-based;
   7. `pending.complete`;
   8. `tx.commit()`.
<!-- rusticate:module(postgres):end -->
<!-- rusticate:module(messaging):start -->
   With messaging, publish with `tx.emit(&Event)` before `pending.complete`, in the same transaction (MSG-001).
<!-- rusticate:module(messaging):end -->
4. **Adapter** in `http.rs`.
   - Write `*Request` with `deny_unknown_fields` and a `TryFrom<Request> for Cmd` that collects **all** violations with JSON paths (`lines[2].quantity`).
   - Write the `*Response` DTO and the `#[utoipa::path]` with an explicit camelCase `operation_id`, every status code, and `security(("bearer" = []))`.
   - Return 201 + `Location` on create.
<!-- rusticate:module(postgres):start -->
   - With PostgreSQL, also `ETag` on create, read and update.
<!-- rusticate:module(postgres):end -->
5. **Error mapping.** Map each new `<Ctx>Error` variant or engine error to a status and stable `error_code` in `error.rs` (API-002), and add it to the mapping test.

## 5. Tests that prove behaviour

In `tests/integration/<feature>.rs`, each test is Arrange–Act–Assert with a behavioural name:

- happy path: assert the returned value;
- every failure mode the workflow can return;
- `*_requires_its_permission`;
- **tenant isolation**: a second tenant cannot read or change it (expect `NotFound`).
<!-- rusticate:module(postgres):start -->

With PostgreSQL, also:

- the happy path reads back what was stored;
- commands: idempotent replay returns the same result with no second side effect; key reuse with another body is rejected;
- updates: a stale version is rejected; concurrent updates have exactly one winner (`tokio::join!`).
<!-- rusticate:module(postgres):end -->
<!-- rusticate:module(messaging):start -->
- events: exactly one outbox row of the right type.
<!-- rusticate:module(messaging):end -->

Add an HTTP-level test in `apps/api/tests/` when the endpoint has HTTP-specific behaviour (headers, status codes, pagination).

## 6. Contract and finish

```sh
cargo rusticate openapi        # review the diff of apps/api/openapi.json like code
cargo rusticate verify         # the definition of done
```

Then self-review with [review-change](../review-change/SKILL.md).

## Pitfalls

- **Business decisions in adapters** (or in `db.rs`). Status transitions, totals and limits belong in the engine or the workflow.
- **Trusting the client for server facts** (prices, tax, currency, tenant id). Load them.
- **Returning `ApiError` from a workflow.** Workflows are also called by tests, consumers and jobs.
<!-- rusticate:module(postgres):start -->
- **`fetch_all` without `LIMIT`**, or `($1 IS NULL OR col = $1)` filters. Write one query per filter shape (PERF-005).
- **Calling another feature's `db` module.** Use its `pub(crate) *_in_tx` helper (same context), or events (other contexts).
<!-- rusticate:module(postgres):end -->
