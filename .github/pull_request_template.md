## What and why

<!-- The use case in the ubiquitous language, and the reason for the change. Link the PRD/TRD or issue. -->

## How

<!-- Context(s) touched, new features/migrations/events, notable decisions (link an ADR if one was needed). -->

## Checklist

`cargo xtask verify` passed locally, and CI is green. CI enforces these items too; the list is for the reviewer.

- [ ] Feature slices follow `<feature>.rs` / `http.rs` / `db.rs` (ARCH-004); no `SCAFFOLD:` markers remain
- [ ] Every workflow checks its permission first and takes `ctx: &OpCtx` (SEC-003)
- [ ] Integration tests cover the happy path, each failure mode, **tenant isolation**, and idempotent replay where applicable (TEST-002/005)
- [ ] New business rules live in the engine with property tests against an independent oracle (ENG-005)
- [ ] Migrations follow the tenant-table template; no edits to merged migrations (DB-004, DB-011)
- [ ] `.sqlx/` and `apps/api/openapi.json` regenerated and the diff reviewed (DB-010, API-004)
- [ ] Events are new versions, not edits of published ones, and carry no personal data (MSG-003/006)
- [ ] No `#[allow]`; every `#[expect]` has a reason (LINT-002)

## Rollout

<!-- Backwards-compatible with the running version? Migration expand/contract steps? Feature flag? Consumers to update? -->
