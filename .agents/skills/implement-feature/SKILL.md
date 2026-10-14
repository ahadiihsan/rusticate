---
name: implement-feature
description: Turn a PRD, TRD, ticket or feature request into a finished feature slice in this Rust workspace (workflow, HTTP or event adapter, SQL, engine rules, tests, contract). Use for any new endpoint, use case or behaviour change in a bounded context.
---

# Implement a feature

The goal is a change that passes `cargo xtask verify` and that a reviewer can read like the [`billing`](../../../crates/domains/billing) reference. Rules are cited by ID; their reasons are in the [SOP](../../../docs/sop/rust-backend-sop-v2.md).

## 1. Classify before writing code

Answer these in your notes or PR description:

| Question | Decides |
|---|---|
| Which bounded context owns this, in its own words? | the crate; a *new* context needs [new-bounded-context](../new-bounded-context/SKILL.md) |
| Does it change state (command), read state (query), or react to an event? | `--kind command\|query\|event` |
| Name it `<verb>_<noun>` in the domain language (`issue_invoice`, not `create_invoice_record`) | module, permission `<ctx>.<noun>.<verb>`, route |
| Which business rules or calculations? | engine functions ([write-engine-rules](../write-engine-rules/SKILL.md)) |
| Which data does the server decide (prices, rates, limits), never the client? | what the workflow loads instead of accepting |
| External effect (money, stock, messages)? | `Idempotency-Key` (API-005) |
| Updates an existing aggregate? | `version` + `If-Match` (API-007, DB-008) |
| Can it touch unbounded rows? | keyset pagination or a batch job (DB-007) |
| Should other contexts know it happened? | an event ([add-event-consumer](../add-event-consumer/SKILL.md)) |
| Does it handle personal data? | `Sensitive<T>`, no personal data in events or logs (SEC-006, MSG-006) |

## 2. Schema (only if tables change)

Follow [add-migration](../add-migration/SKILL.md): `cargo xtask new migration <ctx> "<what>"`, fill the tenant template, then `cargo xtask db migrate`.

## 3. Business rules first

Write pure functions in `<ctx>-engine`, with error variants and with example and property tests. Run `cargo xtask test -p <ctx>-engine`. This is the cheapest place to get the logic right.

## 4. Scaffold the slice

```sh
cargo xtask new feature <ctx> <verb_noun> --kind command   # or query / event
```

This creates `<feature>.rs`, `<feature>/http.rs` (or `handler.rs`), `<feature>/db.rs` and `tests/integration/<feature>.rs`. It also adds the permission constant, mounts the route (or registers the consumer in the worker), and runs `cargo fmt`. `cargo xtask arch` lists every `SCAFFOLD:` marker; replace them **in this order**:

1. **Command / view types** in `<feature>.rs`. Fields are value objects (`CustomerEmail`, `Quantity`, `InvoiceId`), never raw strings or numbers. Add new value objects to `types.rs` with a `parse` constructor.
2. **SQL** in `<feature>/db.rs`. Use the `query!` family against `&mut TenantTx`, with `WHERE tenant_id = $1` on every statement. Return rows; convert to domain types in the workflow. Then run `cargo xtask db prepare`.
3. **Workflow** in `<feature>.rs`. Follow [`issue_invoice`](../../../crates/domains/billing/src/issue_invoice.rs) step by step:
   1. `ctx.require(permissions::X)?`;
   2. for commands, `Fingerprint::of` and `db.begin(ctx)`; for queries, `db.begin_read_only(ctx)`;
   3. `idempotency::claim` (replay returns early);
   4. load server-side facts;
   5. call the engine;
   6. persist set-based;
   7. `tx.emit(&Event)`;
   8. `pending.complete`;
   9. `tx.commit()`.

   Return the context error enum.
4. **Adapter** in `http.rs`.
   - Write `*Request` with `deny_unknown_fields` and a `TryFrom<Request> for Cmd` that collects **all** violations with JSON paths (`lines[2].quantity`).
   - Write the `*Response` DTO and the `#[utoipa::path]` with an explicit camelCase `operation_id`, every status code, and `security(("bearer" = []))`.
   - Return 201 + `Location` + `ETag` on create, and `ETag` on read/update.
5. **Error mapping.** Map each new `<Ctx>Error` variant or engine error to a status and stable `error_code` in `error.rs` (API-002), and add it to the mapping test.

## 5. Tests that prove behaviour

In `tests/integration/<feature>.rs`, each test is Arrange–Act–Assert with a behavioural name:

- happy path: assert the returned value **and** that it was stored (read it back);
- every failure mode the workflow can return;
- `*_requires_its_permission`;
- **tenant isolation**: a second tenant cannot read or change it (expect `NotFound`);
- commands: idempotent replay returns the same result with no second side effect; key reuse with another body is rejected;
- updates: a stale version is rejected; concurrent updates have exactly one winner (`tokio::join!`);
- events: exactly one outbox row of the right type.

Add an HTTP-level case in [`apps/api/tests/api/http_flows.rs`](../../../apps/api/tests/api/http_flows.rs) when the endpoint has HTTP-specific behaviour (headers, status codes, pagination).

## 6. Contract and finish

```sh
cargo xtask openapi        # review the diff of apps/api/openapi.json like code
cargo xtask verify         # the definition of done
```

Then self-review with [review-change](../review-change/SKILL.md).

## Pitfalls

- **Business decisions in `db.rs` or `http.rs`.** Status transitions, totals and limits belong in the engine or the workflow.
- **Trusting the client for server facts** (prices, tax, currency, tenant id). Load them.
- **`fetch_all` without `LIMIT`**, or `($1 IS NULL OR col = $1)` filters. Write one query per filter shape (PERF-005).
- **Calling another feature's `db` module.** Use its `pub(crate) *_in_tx` helper (same context), or events (other contexts).
- **Returning `ApiError` from a workflow.** Workflows are also called by consumers and jobs.
