---
name: review-change
description: Review a change in this Rust workspace — your own before declaring a task done, or someone else's PR — against the SOP rules machines cannot fully check (correctness, tenancy, idempotency, contracts, test quality). Use as the last step of every implementation task.
---

# Review a change

`cargo rusticate verify` proves the mechanical rules. This review covers what it cannot. Read the diff top to bottom; for each item, either confirm it or fix it.

## 1. The checks actually ran

- [ ] `cargo rusticate verify` passed **after** the last edit (not `--fast` only).
- [ ] No `SCAFFOLD:` markers, no `#[expect]` without a convincing reason, no deleted or weakened tests.
- [ ] The `apps/api/openapi.json` diff is present if the API changed, and it matches the intent.
<!-- rusticate:module(postgres):start -->
- [ ] Each changed crate's `.sqlx/` diff is present if its queries changed.
<!-- rusticate:module(postgres):end -->

## 2. Correctness

- [ ] The workflow does what the requirement says, in its words (names follow the ubiquitous language).
- [ ] Server-decided facts (prices, rates, currency, tenant, actor) are loaded, never taken from the request.
- [ ] Every error path returns a precise variant with a stable `error_code` and the right status (API-002). No generic 500 for an expected situation.
- [ ] Business rules live in the engine; adapters (and `db.rs`) contain no decisions.
- [ ] Money is integer minor units with a currency; rounding happens once, half-to-even.

## 3. Tenancy and security

- [ ] `ctx.require(PERMISSION)?` is the **first** statement, and the permission is the narrowest that fits.
<!-- rusticate:module(postgres):start -->
- [ ] Every query includes `tenant_id = $1`. RLS is the backstop, not the filter.
- [ ] New tables follow the tenant template.
<!-- rusticate:module(postgres):end -->
- [ ] A test shows another tenant cannot read or change the data.
- [ ] Personal data is `Sensitive<T>` in domain types, absent from events, logs and span fields.
- [ ] No secrets or credentials anywhere in the diff.

## 4. Consistency under failure and concurrency

- [ ] Retrying any operation is safe: the client can resend without a second effect.
<!-- rusticate:module(postgres):start -->
- [ ] State, events, idempotency keys and inbox records commit in **one** transaction.
- [ ] Commands with external effects take `IdempotencyKey`, and retries replay.
- [ ] Updates are version-guarded (If-Match), and the concurrent test shows exactly one winner.
- [ ] Nothing does I/O (HTTP calls, broker publishes) while holding a transaction open.
<!-- rusticate:module(postgres):end -->
<!-- rusticate:module(messaging):start -->
- [ ] Consumers are idempotent and order-tolerant.
<!-- rusticate:module(messaging):end -->

## 5. Performance

- [ ] No unnecessary clones of large values; no blocking calls on the async runtime.
<!-- rusticate:module(postgres):start -->
- [ ] No query per item (N+1); writes of N rows are one `UNNEST` statement.
- [ ] Reads are bounded (`LIMIT`, keyset pagination); there are no `OFFSET`s.
- [ ] Filters do not use `($x IS NULL OR …)`. New query shapes have a supporting index that starts with `tenant_id`.
- [ ] Read-only workflows use `begin_read_only`.
<!-- rusticate:module(postgres):end -->

## 6. Contracts and evolution

- [ ] API changes are backwards compatible, or a new major version is introduced. Documented responses match reality.
<!-- rusticate:module(messaging):start -->
- [ ] Event changes are new versions, not edits.
<!-- rusticate:module(messaging):end -->
<!-- rusticate:module(postgres):start -->
- [ ] Migrations are expand/contract-safe against the currently running version.
<!-- rusticate:module(postgres):end -->

## 7. Tests are evidence

- [ ] Tests assert outcomes (stored state, returned values, emitted events), not just "no error".
- [ ] Each failure mode the code can return has a test.
- [ ] Names state behaviour, and each test is Arrange–Act–Assert.

Report what you verified and anything you could not verify, plainly. A review that says "looks good" without evidence is not a review.
