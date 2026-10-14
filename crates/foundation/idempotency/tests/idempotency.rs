//! Same-transaction idempotency semantics against a real database.
#![cfg(test)]

use std::time::Duration;

use idempotency::{Claim, Fingerprint, IdempotencyError, IdempotencyKey, claim};
use op_context::OpCtx;
use serde::{Deserialize, Serialize};
use sqlx::postgres::{PgConnectOptions, PgPoolOptions};
use test_kit::{TestDb, ctx};

const OP: &str = "test.thing.create";

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
struct Created {
    id: u32,
}

fn key(raw: &str) -> IdempotencyKey {
    IdempotencyKey::parse(raw).unwrap()
}

fn fp(n: u32) -> Fingerprint {
    Fingerprint::of(OP, &serde_json::json!({ "n": n })).unwrap()
}

/// Runs claim → complete → commit; returns what the caller would respond with.
async fn run_once(db: &TestDb, user: &OpCtx, k: &str, n: u32) -> Result<Created, IdempotencyError> {
    let mut tx = db.app.begin(user).await.unwrap();
    let result = match claim::<Created>(&mut tx, OP, &key(k), &fp(n)).await? {
        Claim::Replay(previous) => previous,
        Claim::Fresh(pending) => {
            let created = Created { id: n };
            pending.complete(&mut tx, &created).await?;
            created
        }
    };
    tx.commit().await.unwrap();
    Ok(result)
}

#[sqlx::test(migrations = false)]
async fn replays_the_first_result_for_the_same_payload(
    pool: PgPoolOptions,
    conn: PgConnectOptions,
) {
    let db = TestDb::new(pool, conn, &[]).await;
    let user = ctx::user(ctx::tenant(), &[]);

    let first = run_once(&db, &user, "k1", 7).await.unwrap();
    let second = run_once(&db, &user, "k1", 7).await.unwrap();

    assert_eq!(first, second);
}

#[sqlx::test(migrations = false)]
async fn rejects_key_reuse_with_a_different_payload(pool: PgPoolOptions, conn: PgConnectOptions) {
    let db = TestDb::new(pool, conn, &[]).await;
    let user = ctx::user(ctx::tenant(), &[]);
    run_once(&db, &user, "k1", 7).await.unwrap();

    let reused = run_once(&db, &user, "k1", 8).await;

    assert!(matches!(reused, Err(IdempotencyError::KeyReused)));
}

#[sqlx::test(migrations = false)]
async fn keys_are_scoped_per_actor_and_tenant(pool: PgPoolOptions, conn: PgConnectOptions) {
    let db = TestDb::new(pool, conn, &[]).await;
    let tenant = ctx::tenant();
    run_once(&db, &ctx::user(tenant, &[]), "shared", 1)
        .await
        .unwrap();

    let other_actor = run_once(&db, &ctx::user(tenant, &[]), "shared", 2)
        .await
        .unwrap();
    let other_tenant = run_once(&db, &ctx::user(ctx::tenant(), &[]), "shared", 3)
        .await
        .unwrap();

    assert_eq!(other_actor, Created { id: 2 });
    assert_eq!(other_tenant, Created { id: 3 });
}

#[sqlx::test(migrations = false)]
async fn a_rolled_back_claim_leaves_no_trace(pool: PgPoolOptions, conn: PgConnectOptions) {
    let db = TestDb::new(pool, conn, &[]).await;
    let user = ctx::user(ctx::tenant(), &[]);
    let mut tx = db.app.begin(&user).await.unwrap();
    let Claim::Fresh(_pending) = claim::<Created>(&mut tx, OP, &key("k1"), &fp(1))
        .await
        .unwrap()
    else {
        panic!("first claim must be fresh");
    };
    drop(tx); // the workflow failed: rollback

    let retried = run_once(&db, &user, "k1", 2).await.unwrap();

    assert_eq!(
        retried,
        Created { id: 2 },
        "a failed attempt must not poison the key"
    );
}

#[sqlx::test(migrations = false)]
async fn expired_keys_are_reclaimed(pool: PgPoolOptions, conn: PgConnectOptions) {
    let db = TestDb::new(pool, conn, &[]).await;
    let user = ctx::user(ctx::tenant(), &[]);
    run_once(&db, &user, "k1", 1).await.unwrap();
    sqlx::raw_sql(
        "ALTER TABLE platform.idempotency_keys NO FORCE ROW LEVEL SECURITY; \
                   UPDATE platform.idempotency_keys SET expires_at = now() - interval '1 second'; \
                   ALTER TABLE platform.idempotency_keys FORCE ROW LEVEL SECURITY;",
    )
    .execute(db.owner_pool())
    .await
    .unwrap();

    let after_expiry = run_once(&db, &user, "k1", 2).await.unwrap();

    assert_eq!(after_expiry, Created { id: 2 });
}

#[sqlx::test(migrations = false)]
async fn a_concurrent_duplicate_waits_then_replays(pool: PgPoolOptions, conn: PgConnectOptions) {
    let db = TestDb::new(pool, conn, &[]).await;
    let user = ctx::user(ctx::tenant(), &[]);
    let mut first = db.app.begin(&user).await.unwrap();
    let Claim::Fresh(pending) = claim::<Created>(&mut first, OP, &key("k1"), &fp(1))
        .await
        .unwrap()
    else {
        panic!("first claim must be fresh");
    };

    let duplicate = {
        let app = db.app.clone();
        let user = user.clone();
        tokio::spawn(async move {
            let mut tx = app.begin(&user).await.unwrap();
            let outcome = claim::<Created>(&mut tx, OP, &key("k1"), &fp(1)).await;
            tx.commit().await.unwrap();
            outcome
        })
    };
    tokio::time::sleep(Duration::from_millis(200)).await;
    assert!(
        !duplicate.is_finished(),
        "the duplicate must block on the first claim"
    );
    pending
        .complete(&mut first, &Created { id: 1 })
        .await
        .unwrap();
    first.commit().await.unwrap();

    let outcome = duplicate.await.unwrap().unwrap();

    assert!(matches!(outcome, Claim::Replay(Created { id: 1 })));
}

#[sqlx::test(migrations = false)]
async fn a_duplicate_that_waits_too_long_is_in_flight(pool: PgPoolOptions, conn: PgConnectOptions) {
    let db = TestDb::new(pool, conn, &[]).await;
    let user = ctx::user(ctx::tenant(), &[]);
    let mut first = db.app.begin(&user).await.unwrap();
    let Claim::Fresh(_pending) = claim::<Created>(&mut first, OP, &key("k1"), &fp(1))
        .await
        .unwrap()
    else {
        panic!("first claim must be fresh");
    };

    let mut second = db.app.begin(&user).await.unwrap();
    sqlx::query("SET LOCAL lock_timeout = '100ms'")
        .execute(second.conn())
        .await
        .unwrap();
    let outcome = claim::<Created>(&mut second, OP, &key("k1"), &fp(1)).await;

    assert!(matches!(outcome, Err(IdempotencyError::InFlight)));
}
