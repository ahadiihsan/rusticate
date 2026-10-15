# rusticate

A production Rust backend, with heavy support for multi-tenancy. You get the standard, the scaffolding and the enforcement together:

- **[Engineering standard (SOP v2)](docs/sop/rust-backend-sop-v2.md):** rules with IDs, each with its reason and its enforcer.
- **Scaffolding:** `cargo rusticate new domain|feature` generates code that already follows every rule.
<!-- rusticate:module(example):start -->
- **Example:** the [`billing`](crates/domains/billing) bounded context, complete and tested, built on the platform crates in [`crates/foundation`](crates/foundation).
<!-- rusticate:module(example):end -->
- **Enforcement:** the compiler, clippy, `cargo rusticate arch`, `cargo-deny` and the tests. `cargo rusticate verify` is the definition of done.
- **Agent harness:** [AGENTS.md](AGENTS.md), [task skills](.agents/skills), hooks.

Stack: Rust 2024 · Axum · OpenTelemetry · OpenAPI 3.1.
<!-- rusticate:module(postgres):start -->
With PostgreSQL 18: SQLx, row-level security for tenant isolation, a catalog guard and squawk for migrations.
<!-- rusticate:module(postgres):end -->
<!-- rusticate:module(messaging):start -->
With NATS JetStream: a worker, a transactional outbox and idempotent event consumers.
<!-- rusticate:module(messaging):end -->

<!-- rusticate:template-only:start -->
## Start a new project

```sh
cargo install cargo-rusticate --locked
cargo rusticate new my-service --owner <github-user-or-org>
```

The generator asks which optional modules to include. Each one builds on the one before it:

| Module | Adds |
|---|---|
| core (always) | HTTP API, auth, problem+json errors, telemetry, scaffolding, CI, agent harness |
| `postgres` | PostgreSQL with row-level security (multi-tenant), migrations, idempotency |
| `messaging` | NATS JetStream: worker, transactional outbox, event consumers |
| `example` | the `billing` context, a complete reference |

To answer without questions, pass `--minimal` (core only), `--with <modules>`, `--without <modules>`, `--bare` (everything but the example) or `--all`.

Already created the repository on GitHub? Clone it and fill it in place (the directory may contain only `.git`):

```sh
git clone git@github.com:<owner>/my-service.git && cd my-service
cargo rusticate init --owner <owner>
```

The generator copies this template at the release matching its version, keeps the modules you chose, renames everything for your project, and starts a fresh git history. See [cargo-rusticate](cargo-rusticate/README.md).

<!-- rusticate:template-only:end -->
## Quickstart

Requirements: rustup, and `cargo install cargo-rusticate --locked`. Every `cargo rusticate <command>` runs this project's own `cargo xtask <command>`, which also works without it.
<!-- rusticate:module(postgres):start -->
Docker runs the local services.
<!-- rusticate:module(postgres):end -->

```sh
cargo rusticate setup        # git hooks, missing tools, formatting, API contract
```

<!-- rusticate:module(postgres):start -->
```sh
cargo rusticate db up        # local Postgres :55432 (with the role contract)
cargo rusticate db migrate
```

<!-- rusticate:module(postgres):end -->
```sh
cargo rusticate verify       # fmt, architecture, lint, tests
cargo rusticate run api      # http://localhost:8080/docs
```

A debug build of this stack takes a few GB in `target/`. With several projects, share one build cache so dependencies compile once: `build.target-dir = "/path/to/shared-target"` in `~/.cargo/config.toml`.

<!-- rusticate:module(messaging):start -->
```sh
cargo rusticate run worker   # second terminal: outbox relay + consumers
```

<!-- rusticate:module(messaging):end -->
<!-- rusticate:module(example):start -->
Try the example:

```sh
TOKEN=$(cargo rusticate token --perm billing.settings.manage --perm billing.invoice.issue --perm billing.invoice.read)
curl -s -X PUT localhost:8080/v1/billing/settings -H "authorization: Bearer $TOKEN" \
  -H 'if-none-match: *' -H 'content-type: application/json' -d '{"currency":"IDR","tax_rate_bps":1100}'
curl -s -X POST localhost:8080/v1/invoices -H "authorization: Bearer $TOKEN" \
  -H "idempotency-key: $(uuidgen)" -H 'content-type: application/json' \
  -d '{"customer_email":"ap@customer.example","lines":[{"description":"Consulting","quantity":2,"unit_price_minor":15000000}]}'
```

<!-- rusticate:module(example):end -->
## Build something

```sh
cargo rusticate new domain shipping                              # bounded context + engine, wired in
cargo rusticate new feature shipping dispatch_parcel --kind command
cargo rusticate arch                                             # lists every SCAFFOLD marker left to fill
```

<!-- rusticate:module(postgres):start -->
With PostgreSQL, the context owns a schema and features get a private `db.rs`; add tables with `cargo rusticate new migration shipping "create shipments"`.

<!-- rusticate:module(postgres):end -->
The [implement-feature](.agents/skills/implement-feature/SKILL.md) skill walks through it.

## Layout

| Path | What |
|---|---|
| `apps/` | deployable binaries (wiring only) |
| `crates/foundation/*` | platform crates: errors, auth context, HTTP runtime, telemetry, and those of the modules in use |
| `crates/domains/*` | bounded contexts and their pure engines |
| `xtask/` | enforcement, scaffolding, automation |
| `architecture.toml` | machine-readable architecture |
| `docs/` | [SOP](docs/sop/rust-backend-sop-v2.md), [ADRs](docs/adr), [configuration](docs/configuration.md), [operations](docs/operations.md) |
| `deploy/k8s/`, `Dockerfile` | reference deployment |
<!-- rusticate:template-only:start -->

## License

Licensed under either of [Apache License, Version 2.0](LICENSE-APACHE) or [MIT license](LICENSE-MIT) at your option.

Unless you explicitly state otherwise, any contribution intentionally submitted for inclusion in this work by you, as defined in the Apache-2.0 license, shall be dual licensed as above, without any additional terms or conditions.
<!-- rusticate:template-only:end -->
