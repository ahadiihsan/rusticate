//! Outbox relay, consumer dedup and maintenance against a real database.
#![cfg(test)]

use std::time::Duration;

use datastore::{DomainEvent, TenantTx};
use messaging::{
    EventHandler, HandlerError, InboundEvent, MemorySink, OutboxRelay, Outcome, RelayConfig,
    RelayError, maintenance, process,
};
use op_context::{CorrelationId, OpCtx, Permission};
use serde::Serialize;
use sqlx::postgres::{PgConnectOptions, PgPoolOptions};
use test_kit::{TestDb, ctx};
use tokio_util::sync::CancellationToken;
use uuid::Uuid;

#[derive(Serialize)]
struct Pinged {
    n: u32,
}

impl DomainEvent for Pinged {
    const EVENT_TYPE: &'static str = "test.ping.pinged.v1";
    const AGGREGATE_TYPE: &'static str = "test.ping";

    fn aggregate_id(&self) -> Uuid {
        Uuid::from_u128(u128::from(self.n))
    }
}

async fn emit(db: &TestDb, user: &OpCtx, n: u32) -> Uuid {
    let mut tx = db.app.begin(user).await.unwrap();
    let id = tx.emit(&Pinged { n }).await.unwrap();
    tx.commit().await.unwrap();
    id
}

async fn outbox_len(db: &TestDb) -> i64 {
    let mut tx = db.system.begin().await.unwrap();
    sqlx::query_scalar("SELECT count(*) FROM platform.outbox_events")
        .fetch_one(tx.conn())
        .await
        .unwrap()
}

fn relay(db: &TestDb, sink: &MemorySink) -> OutboxRelay<MemorySink> {
    OutboxRelay::new(db.system.clone(), sink.clone(), RelayConfig::default())
}

#[sqlx::test(migrations = false)]
async fn relay_publishes_in_order_then_deletes(pool: PgPoolOptions, conn: PgConnectOptions) {
    let db = TestDb::new(pool, conn, &[]).await;
    let user = ctx::user(ctx::tenant(), &[]);
    let ids = [
        emit(&db, &user, 1).await,
        emit(&db, &user, 2).await,
        emit(&db, &user, 3).await,
    ];
    let sink = MemorySink::default();

    let published = relay(&db, &sink).relay_batch().await.unwrap();

    assert_eq!(published, 3);
    let relayed: Vec<Uuid> = sink.published().iter().map(|e| e.id).collect();
    assert_eq!(relayed, ids);
    assert_eq!(outbox_len(&db).await, 0);
}

#[sqlx::test(migrations = false)]
async fn a_failed_publish_loses_nothing(pool: PgPoolOptions, conn: PgConnectOptions) {
    let db = TestDb::new(pool, conn, &[]).await;
    emit(&db, &ctx::user(ctx::tenant(), &[]), 1).await;
    let sink = MemorySink::default();
    sink.set_failing(true);

    assert!(relay(&db, &sink).relay_batch().await.is_err());
    assert_eq!(outbox_len(&db).await, 1);

    sink.set_failing(false);
    assert_eq!(relay(&db, &sink).relay_batch().await.unwrap(), 1);
    assert_eq!(outbox_len(&db).await, 0);
}

#[sqlx::test(migrations = false)]
async fn only_one_relay_is_active_at_a_time(pool: PgPoolOptions, conn: PgConnectOptions) {
    let db = TestDb::new(pool, conn, &[]).await;
    emit(&db, &ctx::user(ctx::tenant(), &[]), 1).await;
    let mut leader = db.system.begin().await.unwrap();
    sqlx::query(
        "SELECT pg_advisory_xact_lock(x'6f757462' :: bigint << 32 | x'6f780001' :: bigint)",
    )
    .execute(leader.conn())
    .await
    .unwrap();
    let sink = MemorySink::default();

    let published_by_standby = relay(&db, &sink).relay_batch().await.unwrap();

    assert_eq!(published_by_standby, 0);
    assert!(sink.published().is_empty());
}

/// Handler whose effect is a follow-up outbox event, so effects are countable.
struct Echo;

impl EventHandler for Echo {
    const CONSUMER: &'static str = "test-echo";
    const EVENT_TYPES: &'static [&'static str] = &["test.ping.pinged.v1"];
    const PERMISSIONS: &'static [Permission] = &[];

    async fn handle(
        &self,
        _ctx: &OpCtx,
        tx: &mut TenantTx,
        event: &InboundEvent,
    ) -> Result<(), HandlerError> {
        let n: u32 = event.payload["n"]
            .as_u64()
            .and_then(|n| u32::try_from(n).ok())
            .unwrap_or(0);
        tx.emit(&Pinged { n: n + 100 }).await?;
        Ok(())
    }
}

struct AlwaysFails;

impl EventHandler for AlwaysFails {
    const CONSUMER: &'static str = "test-always-fails";
    const EVENT_TYPES: &'static [&'static str] = &["test.ping.pinged.v1"];
    const PERMISSIONS: &'static [Permission] = &[];

    async fn handle(
        &self,
        _ctx: &OpCtx,
        tx: &mut TenantTx,
        _event: &InboundEvent,
    ) -> Result<(), HandlerError> {
        tx.emit(&Pinged { n: 999 }).await?;
        Err(HandlerError::permanent(std::io::Error::other(
            "invariant violated",
        )))
    }
}

fn inbound(tenant: op_context::TenantId) -> InboundEvent {
    InboundEvent {
        id: Uuid::now_v7(),
        event_type: "test.ping.pinged.v1".to_owned(),
        tenant_id: tenant,
        correlation_id: CorrelationId::new_v7(),
        aggregate_id: Uuid::now_v7(),
        traceparent: None,
        payload: serde_json::json!({ "n": 1 }),
    }
}

#[sqlx::test(migrations = false)]
async fn redelivered_events_take_effect_once(pool: PgPoolOptions, conn: PgConnectOptions) {
    let db = TestDb::new(pool, conn, &[]).await;
    let event = inbound(ctx::tenant());

    let first = process(&db.app, &Echo, &event).await.unwrap();
    let redelivery = process(&db.app, &Echo, &event).await.unwrap();

    assert_eq!(first, Outcome::Processed);
    assert_eq!(redelivery, Outcome::Duplicate);
    assert_eq!(
        outbox_len(&db).await,
        1,
        "the effect must happen exactly once"
    );
}

#[sqlx::test(migrations = false)]
async fn a_failed_handler_rolls_back_effects_and_inbox(
    pool: PgPoolOptions,
    conn: PgConnectOptions,
) {
    let db = TestDb::new(pool, conn, &[]).await;
    let event = inbound(ctx::tenant());

    let failed = process(&db.app, &AlwaysFails, &event).await;
    let retried_by_fixed_handler = process(&db.app, &Echo, &event).await.unwrap();

    assert!(matches!(failed, Err(HandlerError::Permanent(_))));
    assert_eq!(
        outbox_len(&db).await,
        1,
        "only the successful handler's effect remains"
    );
    assert_eq!(retried_by_fixed_handler, Outcome::Processed);
}

#[sqlx::test(migrations = false)]
async fn maintenance_purges_only_expired_rows(pool: PgPoolOptions, conn: PgConnectOptions) {
    let db = TestDb::new(pool, conn, &[]).await;
    let user = ctx::user(ctx::tenant(), &[]);
    let mut tx = db.app.begin(&user).await.unwrap();
    for (key, ttl) in [("expired", "-1 hour"), ("live", "1 hour")] {
        sqlx::query(
            "INSERT INTO platform.idempotency_keys (actor_id, operation, idempotency_key, fingerprint, expires_at) \
             VALUES ($1, 'op', $2, decode(repeat('00', 32), 'hex'), now() + $3::interval)",
        )
        .bind(user.actor_id().as_uuid())
        .bind(key)
        .bind(ttl)
        .execute(tx.conn())
        .await
        .unwrap();
    }
    tx.commit().await.unwrap();

    let removed = maintenance::purge_once(&db.system).await.unwrap();

    assert_eq!(removed, 1);
}

#[sqlx::test(migrations = false)]
async fn relay_loop_recovers_from_broker_outage_and_stops_on_shutdown(
    pool: PgPoolOptions,
    conn: PgConnectOptions,
) {
    let db = TestDb::new(pool, conn, &[]).await;
    let user = ctx::user(ctx::tenant(), &[]);
    for n in 0..5 {
        emit(&db, &user, n).await;
    }
    let sink = MemorySink::default();
    sink.set_failing(true);
    let shutdown = CancellationToken::new();
    let config = RelayConfig {
        batch_size: 2,
        idle_poll: Duration::from_millis(10),
        error_backoff: Duration::from_millis(10),
        ..RelayConfig::default()
    };
    let running = tokio::spawn(
        OutboxRelay::new(db.system.clone(), sink.clone(), config).run(shutdown.clone()),
    );

    tokio::time::sleep(Duration::from_millis(50)).await;
    assert!(
        sink.published().is_empty(),
        "nothing is lost while the broker is down"
    );
    sink.set_failing(false);
    for _ in 0..200 {
        if sink.published().len() == 5 {
            break;
        }
        tokio::time::sleep(Duration::from_millis(10)).await;
    }
    shutdown.cancel();
    running.await.unwrap();

    assert_eq!(sink.published().len(), 5);
    assert_eq!(outbox_len(&db).await, 0);
}

#[sqlx::test(migrations = false)]
async fn maintenance_loop_runs_until_shutdown(pool: PgPoolOptions, conn: PgConnectOptions) {
    let db = TestDb::new(pool, conn, &[]).await;
    let shutdown = CancellationToken::new();
    let running = tokio::spawn(maintenance::run(
        db.system.clone(),
        Duration::from_millis(10),
        shutdown.clone(),
    ));

    tokio::time::sleep(Duration::from_millis(40)).await;
    shutdown.cancel();

    tokio::time::timeout(Duration::from_secs(2), running)
        .await
        .expect("maintenance stops promptly on shutdown")
        .unwrap();
}

#[sqlx::test(migrations = false)]
async fn a_slow_broker_times_out_the_batch_without_losing_it(
    pool: PgPoolOptions,
    conn: PgConnectOptions,
) {
    let db = TestDb::new(pool, conn, &[]).await;
    emit(&db, &ctx::user(ctx::tenant(), &[]), 1).await;
    let sink = MemorySink::default();
    sink.set_delay(Duration::from_millis(200));
    let config = RelayConfig {
        publish_timeout: Duration::from_millis(20),
        ..RelayConfig::default()
    };

    let outcome = OutboxRelay::new(db.system.clone(), sink, config)
        .relay_batch()
        .await;

    assert!(matches!(outcome, Err(RelayError::PublishTimeout(_))));
    assert_eq!(
        outbox_len(&db).await,
        1,
        "the batch stays for the next attempt"
    );
}
