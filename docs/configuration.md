# Configuration reference

Binaries read `APP_*` environment variables once at startup (CFG-001); `__` separates nesting levels. Unknown keys are rejected and invalid values stop startup with every problem listed (CFG-002). Local defaults live only in [`xtask/src/dev_env.rs`](../xtask/src/dev_env.rs) and are applied by `cargo rusticate run|test|db`.

## api

| Variable | Default | Notes |
|---|---|---|
| `APP_ENVIRONMENT` | `local` | `local` \| `staging` \| `production`. Outside `local`: no HS256, no docs UI. |
| `APP_DATABASE__URL` | — (required) | `app_rw` role. The startup check refuses superusers, owners and `BYPASSRLS`. |
| `APP_DATABASE__MAX_CONNECTIONS` / `__MIN_CONNECTIONS` | 20 / 2 | per instance; keep `replicas × max` below Postgres capacity (or front with PgBouncer in transaction mode) |
| `APP_DATABASE__ACQUIRE_TIMEOUT_MS` | 3000 | exceeded → 503 `DATABASE_UNAVAILABLE` |
| `APP_DATABASE__STATEMENT_TIMEOUT_MS` / `__LOCK_TIMEOUT_MS` / `__IDLE_IN_TRANSACTION_TIMEOUT_MS` | 5000 / 3000 / 15000 | sent in the connection startup packet |
| `APP_MIGRATION_DATABASE_URL` | — | `app_owner` role; only for `api migrate` |
| `APP_AUTH__ISSUER` / `APP_AUTH__AUDIENCE` | — (required) | expected `iss` / `aud` claims |
| `APP_AUTH__PUBLIC_KEY_PEM` | — | production verification key |
| `APP_AUTH__PUBLIC_KEY_ALGORITHM` | `RS256` | `RS256` \| `ES256` |
| `APP_AUTH__HS256_SECRET` | — | local development only (≥ 32 bytes) |
| `APP_AUTH__LEEWAY_SECS` | 30 | clock skew allowance |
| `APP_HTTP__BIND` | `0.0.0.0:8080` | |
| `APP_HTTP__REQUEST_TIMEOUT_MS` | 10000 | exceeded → 503 `REQUEST_TIMEOUT` |
| `APP_HTTP__BODY_LIMIT_BYTES` | 1048576 | exceeded → 413 |
| `APP_HTTP__MAX_CONCURRENT_REQUESTS` | 512 | beyond it → 503 `OVERLOADED` (load shedding) |
| `APP_HTTP__DRAIN_DELAY_MS` / `__SHUTDOWN_TIMEOUT_MS` | 5000 / 20000 | the sum must stay below the pod grace period |
| `APP_HTTP__CORS_ALLOWED_ORIGINS` | `[]` | e.g. `["https://app.example.com"]`; empty disables CORS |
| `APP_HTTP__DOCS_UI` | `false` | `/docs` (local only); `/openapi.json` is always served |
| `APP_TELEMETRY__LOG_FORMAT` | `json` | `json` \| `pretty` |
| `APP_TELEMETRY__LOG_FILTER` | `info` | `RUST_LOG` overrides it |
| `APP_TELEMETRY__OTLP_ENDPOINT` | — | OTLP/gRPC collector; without it, trace ids exist but are not exported |
| `APP_TELEMETRY__TRACE_SAMPLE_RATIO` | `1.0` | head sampling for root spans |

## worker

| Variable | Default | Notes |
|---|---|---|
| `APP_DATABASE__URL` (+ pool settings above) | — (required) | `app_rw`, for event handlers |
| `APP_SYSTEM_DATABASE__URL` (+ pool settings) | — (required) | `app_system`, for the relay and maintenance |
| `APP_NATS__URL` | — (required) | |
| `APP_NATS__STREAM_REPLICAS` | 1 | 3 in production clusters |
| `APP_CONSUMERS__CONCURRENCY` | 16 | in-flight messages per consumer per replica |
| `APP_CONSUMERS__MAINTENANCE_INTERVAL_SECS` | 60 | retention purge cadence |
| `APP_HTTP__BIND` | `0.0.0.0:8081` | health probes only |
| `APP_TELEMETRY__*` | as api | |

## Local stack

`cargo rusticate db up` starts Postgres 18 on `localhost:55432` with the role contract from [`docker/postgres/initdb/00-roles.sql`](../docker/postgres/initdb/00-roles.sql), and NATS JetStream on `localhost:4222`. The credentials are local-only and equal to the role names.
