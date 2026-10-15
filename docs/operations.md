# Operations

## Deploying a release

<!-- rusticate:module(postgres):start -->
1. **Migrate.** Run the [`migrate` Job](../deploy/k8s/migrate-job.yaml) with the new image (`api migrate`, `app_owner` credentials) and wait for success. Migrations are expand-only relative to the running version (DB-011), so the old pods keep working.
<!-- rusticate:module(postgres):end -->
2. **API.** Rolling update with `maxUnavailable: 0`. New pods receive traffic only when `/health/ready` passes (startup probe), and old pods drain (OPS-003).
<!-- rusticate:module(messaging):start -->
3. **Worker.** Rolling update. The relay leader moves to another replica within one poll interval; consumers rebalance automatically.
<!-- rusticate:module(messaging):end -->
4. **Contract steps** (removing old API fields, columns or event versions) ship in a *later* release, after every client and consumer has moved.

**Rollback:** redeploy the previous image.
<!-- rusticate:module(postgres):start -->
Never roll back a migration; fix forward. Expand-only migrations make the previous version compatible with the new schema.
<!-- rusticate:module(postgres):end -->

## Probes and shutdown

| Endpoint | Meaning | Must not |
|---|---|---|
| `/health/live` | the process is responsive | depend on the database (an outage would restart every pod) |
| `/health/ready` | accepting traffic: all dependency checks pass and not draining | be cached |

On SIGTERM:

1. Readiness turns 503 for `drain_delay` (5 s) while the listener keeps serving.
2. The listener closes and in-flight requests get up to `shutdown_timeout` (20 s).
<!-- rusticate:module(messaging):start -->
3. The worker finishes its current batch or message.
<!-- rusticate:module(messaging):end -->

Keep `terminationGracePeriodSeconds` (60 s) above the sum. No `preStop` hook is needed: the images have no shell.

## What to monitor

| Signal | Source | Alert when |
|---|---|---|
| HTTP 5xx rate and p99 latency per route | request spans (`http.route`, status) | 5xx > 1 % for 5 min; p99 above SLO |
| 503 `OVERLOADED` | problem `error_code` in logs | sustained: scale out |

<!-- rusticate:module(postgres):start -->
With PostgreSQL:

| Signal | Source | Alert when |
|---|---|---|
| 503 `DATABASE_UNAVAILABLE` | problem `error_code` in logs | sustained: raise pool size or check the database |
| RLS or role errors | log `row-level security` / `RoleMisconfigured` | any: treat as a security incident |
| Long transactions | `pg_stat_activity` where `xact_start < now() - interval '1 min'` | any (vacuum stalls, lock contention) |
| Idempotency/inbox table growth | row counts | retention not running (maintenance task) |

<!-- rusticate:module(postgres):end -->
<!-- rusticate:module(messaging):start -->
With messaging:

| Signal | Source | Alert when |
|---|---|---|
| Outbox backlog | `SELECT count(*), min(occurred_at) FROM platform.outbox_events` (as `app_system`) | oldest event > 1 min: relay down or broker unreachable |
| Relay failures | log `outbox relay batch failed` | any, sustained |
| Consumer lag | JetStream consumer `num_pending` per durable | growing for 10 min |
| Terminated (dead-lettered) messages | JetStream advisory `$JS.EVENT.ADVISORY.CONSUMER.MSG_TERMINATED.>` | any |

<!-- rusticate:module(messaging):end -->

Every error response carries `trace_id` and `request_id`. Start any investigation by searching for them.

## Runbooks

<!-- rusticate:module(messaging):start -->
**Outbox backlog growing.**
1. Check worker `/health/ready` (database_system, nats).
2. Check the relay logs.
3. While NATS is down, events accumulate safely and drain on recovery. Publication resumes in id order, and duplicates are deduplicated (`Nats-Msg-Id` within 2 min, then consumer inboxes).

**A message was terminated.** It failed permanently: an undecodable payload or a violated invariant.
1. Find it by `event_id` in the logs.
2. Fix the handler or the data.
3. Republish the event from its source (it is still in the stream for 7 days) or apply the effect with a corrective workflow.

Do not loosen retry limits to make failures disappear.
<!-- rusticate:module(messaging):end -->

<!-- rusticate:module(postgres):start -->
**Suspected cross-tenant exposure.**
1. Confirm the service role posture: `SELECT rolsuper, rolbypassrls FROM pg_roles WHERE rolname = current_user` must return `false, false` for `app_rw`.
2. Run the schema guard (`cargo rusticate test -p api -- schema_guard`) against a restore of production DDL.
3. RLS errors in logs indicate attempted, blocked access, not a leak. Investigate the caller.

**Pool exhaustion (503 `DATABASE_UNAVAILABLE`).**
1. Look for long transactions; `idle_in_transaction_session_timeout` kills leaked ones after 15 s.
2. Look for slow queries (`statement_timeout` 5 s).
3. Look for a traffic spike. Scale horizontally only if Postgres has headroom; otherwise add PgBouncer in transaction mode (transaction-local tenant binding is compatible with it).
<!-- rusticate:module(postgres):end -->

## Repository settings (GitHub)

These live in GitHub, not in the code, so a fork or a fresh copy of this boilerplate starts without them. Apply them once per repository (replace `OWNER/REPO`).

**Branch protection (GOV-002).** The [`main` ruleset](../.github/rulesets/main.json):
- blocks deletion and force pushes;
- requires a pull request, squash-merged, with every conversation resolved;
- requires the CI jobs `verify`, `container images build` and `migrations`, on a branch that is up to date with `main`;
- requires CodeQL results with no new high-severity alert or error;
- lets repository admins bypass only by merging a pull request, never by pushing.

```sh
gh api -X POST repos/OWNER/REPO/rulesets --input .github/rulesets/main.json
```

Required checks match CI **job names**. When you rename a job in [ci.yml](../.github/workflows/ci.yml), update the ruleset in the same change; otherwise the check never reports and every pull request waits forever. With a second maintainer, set `required_approving_review_count` to 1 and `require_code_owner_review` to `true` (nobody can approve their own pull request, so these stay off while there is one maintainer).

**Security and Actions.**

```sh
R=repos/OWNER/REPO
gh api -X PUT $R/vulnerability-alerts                  # Dependabot alerts
gh api -X PUT $R/automated-security-fixes              # Dependabot security PRs
gh api -X PUT $R/private-vulnerability-reporting       # see SECURITY.md
gh api -X PATCH $R/code-scanning/default-setup -f state=configured   # CodeQL: rust, actions
gh api -X PATCH $R -F allow_squash_merge=true -F allow_merge_commit=false \
  -F allow_rebase_merge=false -f squash_merge_commit_title=PR_TITLE \
  -f squash_merge_commit_message=PR_BODY -F delete_branch_on_merge=true \
  -F allow_update_branch=true -F allow_auto_merge=true
gh api -X PUT $R/actions/permissions -F enabled=true -f allowed_actions=selected
gh api -X PUT $R/actions/permissions/selected-actions --input - <<'JSON'
{"github_owned_allowed": true, "verified_allowed": true,
 "patterns_allowed": ["Swatinem/rust-cache@*", "taiki-e/install-action@*"]}
JSON
gh api -X PUT $R/actions/permissions/workflow -f default_workflow_permissions=read \
  -F can_approve_pull_request_reviews=false
gh api -X PUT $R/actions/permissions/fork-pr-contributor-approval \
  -f approval_policy=all_external_contributors
```

Secret scanning with push protection is on by default for public repositories; turn it on under *Settings → Advanced Security* for private ones.

A new third-party action must be added to the allowlist above, and pinned by full commit SHA with its version in a comment (Dependabot keeps both current). SHA pinning is not enforced by the repository setting, because GitHub's own Dependabot workflow references its action by branch.
