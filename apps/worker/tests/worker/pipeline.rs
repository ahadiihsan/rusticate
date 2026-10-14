//! The whole asynchronous pipeline: workflow → outbox → relay → JetStream →
//! consumer → read model, including duplicate publication.

use std::time::Duration;

use billing::configure_billing::{ConfigureBillingCmd, Expectation, configure_billing};
use billing::get_billing_stats::get_billing_stats;
use billing::issue_invoice::{IssueInvoiceCmd, NewLine, issue_invoice};
use billing::permissions::USER_GRANTABLE;
use billing::{CustomerEmail, LineDescription, RecordInvoiceStats};
use billing_engine::invoice::Quantity;
use billing_engine::money::{Bps, Currency};
use idempotency_key::key;
use messaging::nats::{JetStreamSink, Topology, ensure_stream, run_consumer};
use messaging::{OutboxRelay, RelayConfig};
use sqlx::postgres::{PgConnectOptions, PgPoolOptions};
use test_kit::{TestDb, ctx};
use tokio_util::sync::CancellationToken;
use uuid::Uuid;

const NATS_URL: &str = "nats://127.0.0.1:4222";

mod idempotency_key {
    pub(super) fn key(raw: &str) -> idempotency::IdempotencyKey {
        idempotency::IdempotencyKey::parse(raw).unwrap()
    }
}

#[sqlx::test(migrations = false)]
async fn issued_invoice_reaches_the_read_model_exactly_once(
    pool: PgPoolOptions,
    conn: PgConnectOptions,
) {
    let db = TestDb::new(pool, conn, &[&billing::MIGRATOR]).await;
    let nats = async_nats::connect(NATS_URL)
        .await
        .expect("NATS must be running (cargo xtask db up)");
    let js = async_nats::jetstream::new(nats);
    let suffix = Uuid::now_v7().simple().to_string();
    let topology = Topology {
        stream: format!("TEST_{suffix}"),
        subject_prefix: format!("test_{suffix}"),
    };
    let stream = ensure_stream(&js, &topology, 1).await.unwrap();

    let admin = ctx::user(ctx::tenant(), USER_GRANTABLE);
    configure_billing(
        &admin,
        &db.app,
        ConfigureBillingCmd {
            currency: Currency::Idr,
            tax_rate: Bps::new(1_100).unwrap(),
            expectation: Expectation::Create,
        },
    )
    .await
    .unwrap();
    issue_invoice(
        &admin,
        &db.app,
        &key("pipeline-1"),
        IssueInvoiceCmd {
            customer_email: CustomerEmail::parse("ap@customer.example").unwrap(),
            lines: vec![NewLine {
                description: LineDescription::parse("Consulting").unwrap(),
                quantity: Quantity::new(1).unwrap(),
                unit_price_minor: 10_000,
            }],
        },
    )
    .await
    .unwrap();

    let shutdown = CancellationToken::new();
    let consumer = tokio::spawn({
        let (stream, topology, app, shutdown) = (
            stream.clone(),
            topology.clone(),
            db.app.clone(),
            shutdown.clone(),
        );
        async move {
            run_consumer(
                &stream,
                &topology,
                app,
                RecordInvoiceStats,
                std::num::NonZeroUsize::new(4).unwrap(),
                shutdown,
            )
            .await
        }
    });

    // Publish, then simulate a crash-before-delete by publishing the same
    // event again: JetStream dedup (Nats-Msg-Id) and the inbox absorb it.
    let mut tx = db.system.begin().await.unwrap();
    let pending: Vec<messaging::OutboundEvent> = sqlx::query_as!(
        messaging::OutboundEvent,
        r#"SELECT id, tenant_id, event_type, aggregate_type, aggregate_id, payload,
                  correlation_id, traceparent, occurred_at
           FROM platform.outbox_events"#
    )
    .fetch_all(tx.conn())
    .await
    .unwrap();
    drop(tx);
    let sink = JetStreamSink::new(js.clone(), topology.clone());
    messaging::EventSink::publish(&sink, &pending)
        .await
        .unwrap();
    let relay = OutboxRelay::new(db.system.clone(), sink, RelayConfig::default());
    assert_eq!(relay.relay_batch().await.unwrap(), 1);

    let mut stats = Vec::new();
    for _ in 0..100 {
        stats = get_billing_stats(&admin, &db.app).await.unwrap();
        if !stats.is_empty() {
            break;
        }
        tokio::time::sleep(Duration::from_millis(100)).await;
    }
    tokio::time::sleep(Duration::from_millis(300)).await; // let any duplicate arrive
    let stats = get_billing_stats(&admin, &db.app).await.unwrap_or(stats);

    shutdown.cancel();
    consumer.await.unwrap().unwrap();
    js.delete_stream(&topology.stream).await.unwrap();

    assert_eq!(stats.len(), 1, "projection never updated");
    assert_eq!(
        stats[0].issued_count, 1,
        "duplicate publication must not double count"
    );
    assert_eq!(stats[0].issued_total_minor, 11_100);
}
