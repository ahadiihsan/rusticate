# Operations

## Deploying a release

1. **Migrate.** Run the [`migrate` Job](../deploy/k8s/migrate-job.yaml) with the new image (`api migrate`, `app_owner` credentials) and wait for success. Migrations are expand-only relative to the running version (DB-011), so the old pods keep working.
2. **API.** Rolling update with `maxUnavailable: 0`. New pods receive traffic only when `/health/ready` passes (startup probe), and old pods drain (OPS-003).
3. **Worker.** Rolling update. The relay leader moves to another replica within one poll interval; consumers rebalance automatically.
4. **Contract steps** (dropping old columns, old event versions) ship in a *later* release, after every consumer has moved.

**Rollback:** redeploy the previous image. Never roll back a migration; fix forward. Expand-only migrations make the previous version compatible with the new schema.

## Probes and shutdown

| Endpoint | Meaning | Must not |
|---|---|---|
| `/health/live` | the process is responsive | depend on the database (an outage would restart every pod) |
| `/health/ready` | accepting traffic: all dependency checks pass and not draining | be cached |

On SIGTERM:

1. Readiness turns 503 for `drain_delay` (5 s) while the listener keeps serving.
2. The listener closes and in-flight requests get up to `shutdown_timeout` (20 s).
3. The worker finishes its current batch or message.

Keep `terminationGracePeriodSeconds` (60 s) above the sum. No `preStop` hook is needed: the images have no shell.

## What to monitor

| Signal | Source | Alert when |
|---|---|---|
| HTTP 5xx rate and p99 latency per route | request spans (`http.route`, status) | 5xx > 1 % for 5 min; p99 above SLO |
| 503 `OVERLOADED` / `DATABASE_UNAVAILABLE` | problem `error_code` in logs | sustained: scale out or raise pool size |
| Outbox backlog | `SELECT count(*), min(occurred_at) FROM platform.outbox_events` (as `app_system`) | oldest event > 1 min: relay down or broker unreachable |
| Relay failures | log `outbox relay batch failed` | any, sustained |
| Consumer lag | JetStream consumer `num_pending` per durable | growing for 10 min |
| Terminated (dead-lettered) messages | JetStream advisory `$JS.EVENT.ADVISORY.CONSUMER.MSG_TERMINATED.>` | any |
| RLS or role errors | log `row-level security` / `RoleMisconfigured` | any: treat as a security incident |
| Long transactions | `pg_stat_activity` where `xact_start < now() - interval '1 min'` | any (vacuum stalls, lock contention) |
| Idempotency/inbox table growth | row counts | retention not running (maintenance task) |

Every error response carries `trace_id` and `request_id`. Start any investigation by searching for them.

## Runbooks

**Outbox backlog growing.**
1. Check worker `/health/ready` (database_system, nats).
2. Check the relay logs.
3. While NATS is down, events accumulate safely and drain on recovery. Publication resumes in id order, and duplicates are deduplicated (`Nats-Msg-Id` within 2 min, then consumer inboxes).

**A message was terminated.** It failed permanently: an undecodable payload or a violated invariant.
1. Find it by `event_id` in the logs.
2. Fix the handler or the data.
3. Republish the event from its source (it is still in the stream for 7 days) or apply the effect with a corrective workflow.

Do not loosen retry limits to make failures disappear.

**Suspected cross-tenant exposure.**
1. Confirm the service role posture: `SELECT rolsuper, rolbypassrls FROM pg_roles WHERE rolname = current_user` must return `false, false` for `app_rw`.
2. Run the schema guard (`cargo xtask test -p api -- schema_guard`) against a restore of production DDL.
3. RLS errors in logs indicate attempted, blocked access, not a leak. Investigate the caller.

**Pool exhaustion (503 `DATABASE_UNAVAILABLE`).**
1. Look for long transactions; `idle_in_transaction_session_timeout` kills leaked ones after 15 s.
2. Look for slow queries (`statement_timeout` 5 s).
3. Look for a traffic spike. Scale horizontally only if Postgres has headroom; otherwise add PgBouncer in transaction mode (transaction-local tenant binding is compatible with it).
