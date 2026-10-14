---
name: review-change
description: Review a change in this Rust workspace — your own before declaring a task done, or someone else's PR — against the SOP rules machines cannot fully check (correctness, tenancy, idempotency, contracts, test quality). Use as the last step of every implementation task.
---

# Review a change

`cargo xtask verify` proves the mechanical rules. This review covers what it cannot. Read the diff top to bottom; for each item, either confirm it or fix it.

## 1. The checks actually ran

- [ ] `cargo xtask verify` passed **after** the last edit (not `--fast` only).
- [ ] No `SCAFFOLD:` markers, no `#[expect]` without a convincing reason, no deleted or weakened tests.
- [ ] `.sqlx/` and `apps/api/openapi.json` diffs are present if queries or the API changed, and they match the intent.

## 2. Correctness

- [ ] The workflow does what the requirement says, in its words (names follow the ubiquitous language).
- [ ] Server-decided facts (prices, rates, currency, tenant, actor) are loaded, never taken from the request.
- [ ] Every error path returns a precise variant with a stable `error_code` and the right status (API-002). No generic 500 for an expected situation.
- [ ] Business rules live in the engine; `db.rs` and `http.rs` contain no decisions.
- [ ] Money is integer minor units with a currency; rounding happens once, half-to-even.

## 3. Tenancy and security

- [ ] `ctx.require(PERMISSION)?` is the **first** statement, and the permission is the narrowest that fits.
- [ ] Every query includes `tenant_id = $1`. RLS is the backstop, not the filter.
- [ ] New tables follow the tenant template; there is a test showing another tenant cannot read or change the data.
- [ ] Personal data is `Sensitive<T>` in domain types, absent from events, logs and span fields.
- [ ] No secrets or credentials anywhere in the diff.

## 4. Consistency under failure and concurrency

- [ ] State, events, idempotency keys and inbox records commit in **one** transaction.
- [ ] Commands with external effects take `IdempotencyKey`, and retries replay.
- [ ] Updates are version-guarded (If-Match), and the concurrent test shows exactly one winner.
- [ ] Nothing does I/O (HTTP calls, broker publishes) while holding a transaction open.
- [ ] Consumers are idempotent and order-tolerant.

## 5. Performance

- [ ] No query per item (N+1); writes of N rows are one `UNNEST` statement.
- [ ] Reads are bounded (`LIMIT`, keyset pagination); there are no `OFFSET`s.
- [ ] Filters do not use `($x IS NULL OR …)`. New query shapes have a supporting index that starts with `tenant_id`.
- [ ] No unnecessary clones of large values; read-only workflows use `begin_read_only`.

## 6. Contracts and evolution

- [ ] API changes are backwards compatible, or a new major version is introduced. Documented responses match reality.
- [ ] Event changes are new versions, not edits.
- [ ] Migrations are expand/contract-safe against the currently running version.

## 7. Tests are evidence

- [ ] Tests assert outcomes (stored state, returned values, emitted events), not just "no error".
- [ ] Each failure mode the code can return has a test.
- [ ] Names state behaviour, and each test is Arrange–Act–Assert.

Report what you verified and anything you could not verify, plainly. A review that says "looks good" without evidence is not a review.
