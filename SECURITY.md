# Security policy

## Reporting a vulnerability

Please do not open a public issue for a security problem.

Report it privately through GitHub: **Security → Report a vulnerability** on this repository ([direct link](https://github.com/ahadiihsan/rusticate/security/advisories/new)). Include:

- what an attacker can do, and under which conditions;
- steps or a proof of concept that reproduces it;
- the commit you tested.

You will get an acknowledgement within 5 working days. Fixes are developed in a private advisory and published with credit to the reporter, unless you prefer otherwise.

## Supported versions

Only the latest commit on `main` is supported. There are no maintained release branches.

## Scope

In scope: the code in this repository, including the tenant isolation guarantees (row-level security, `TenantTx`), authentication (`iam`), the idempotency and outbox machinery, and the container images built from the `Dockerfile`.

Out of scope: the local development defaults in `xtask/src/dev_env.rs` and `docker-compose.yml` (they are intentionally insecure and never used outside a developer machine), and vulnerabilities in dependencies that are already tracked by a RustSec advisory (`cargo deny` blocks them in CI).
