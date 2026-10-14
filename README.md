# rusticate

A production boilerplate for multi-tenant Rust backends. You get the standard, the reference implementation and the enforcement together:

- **[Engineering standard (SOP v2)](docs/sop/rust-backend-sop-v2.md):** rules with IDs, each with its reason and its enforcer.
- **Reference implementation:** the [`billing`](crates/domains/billing) bounded context, built on the platform crates in [`crates/foundation`](crates/foundation).
- **Enforcement:** the compiler, clippy, `cargo xtask arch`, a PostgreSQL catalog guard, `cargo-deny`, squawk and the tests. `cargo xtask verify` is the definition of done.
- **Agent harness:** [AGENTS.md](AGENTS.md), [task skills](.agents/skills), hooks.

Stack: Rust 2024 · Axum · SQLx on PostgreSQL 18 with row-level security · NATS JetStream · OpenTelemetry · OpenAPI 3.1.

## Quickstart

Requirements: rustup, Docker.

```sh
cargo xtask setup        # git hooks; lists any missing tool and how to install it
cargo xtask db up        # Postgres :55432 (with the role contract) + NATS :4222
cargo xtask db migrate
cargo xtask verify       # fmt, architecture, lint, query metadata, ~150 tests

cargo xtask run api      # http://localhost:8080/docs
cargo xtask run worker   # second terminal: outbox relay + consumers
```

Try it:

```sh
TOKEN=$(cargo xtask token --perm billing.settings.manage --perm billing.invoice.issue --perm billing.invoice.read)
curl -s -X PUT localhost:8080/v1/billing/settings -H "authorization: Bearer $TOKEN" \
  -H 'if-none-match: *' -H 'content-type: application/json' -d '{"currency":"IDR","tax_rate_bps":1100}'
curl -s -X POST localhost:8080/v1/invoices -H "authorization: Bearer $TOKEN" \
  -H "idempotency-key: $(uuidgen)" -H 'content-type: application/json' \
  -d '{"customer_email":"ap@customer.example","lines":[{"description":"Consulting","quantity":2,"unit_price_minor":15000000}]}'
```

## Build something

```sh
cargo xtask new domain shipping                              # bounded context + engine + schema, wired in
cargo xtask new migration shipping "create shipments"
cargo xtask new feature shipping dispatch_parcel --kind command
cargo xtask arch                                             # lists every SCAFFOLD marker left to fill
```

The [implement-feature](.agents/skills/implement-feature/SKILL.md) skill walks through it.

## Layout

| Path | What |
|---|---|
| `apps/api`, `apps/worker` | deployable binaries (wiring only) |
| `crates/foundation/*` | platform: tenancy, errors, idempotency, messaging, HTTP runtime, telemetry, test kit |
| `crates/domains/*` | bounded contexts and their pure engines |
| `xtask/` | enforcement, scaffolding, automation |
| `architecture.toml` | machine-readable architecture |
| `docs/` | [SOP](docs/sop/rust-backend-sop-v2.md), [what changed from v1](docs/sop/v1-to-v2.md), [ADRs](docs/adr), [configuration](docs/configuration.md), [operations](docs/operations.md) |
| `deploy/k8s/`, `Dockerfile` | reference deployment |
