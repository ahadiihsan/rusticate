//! The worker exactly as `main` wires it: relay + every consumer + maintenance.

use std::time::Duration;

use billing::configure_billing::{ConfigureBillingCmd, Expectation, configure_billing};
use billing::get_billing_stats::get_billing_stats;
use billing::issue_invoice::{IssueInvoiceCmd, NewLine, issue_invoice};
use billing::permissions::USER_GRANTABLE;
use billing::{CustomerEmail, LineDescription};
use billing_engine::invoice::Quantity;
use billing_engine::money::{Bps, Currency};
use messaging::nats::{Topology, ensure_stream};
use sqlx::postgres::{PgConnectOptions, PgPoolOptions};
use test_kit::{TestDb, ctx};
use tokio_util::sync::CancellationToken;
use uuid::Uuid;
use worker::tasks::{Deps, spawn_all};

#[sqlx::test(migrations = false)]
async fn spawned_tasks_project_events_and_stop_cleanly(
    pool: PgPoolOptions,
    conn: PgConnectOptions,
) {
    let db = TestDb::new(pool, conn, &[&billing::MIGRATOR]).await;
    let nats = async_nats::connect("nats://127.0.0.1:4222").await.unwrap();
    let js = async_nats::jetstream::new(nats);
    let suffix = Uuid::now_v7().simple().to_string();
    let topology = Topology {
        stream: format!("WIRING_{suffix}"),
        subject_prefix: format!("wiring_{suffix}"),
    };
    let stream = ensure_stream(&js, &topology, 1).await.unwrap();
    let shutdown = CancellationToken::new();
    let mut tasks = spawn_all(
        Deps {
            app: db.app.clone(),
            system: db.system.clone(),
            js: js.clone(),
            stream,
            topology: topology.clone(),
            concurrency: std::num::NonZeroUsize::new(4).unwrap(),
            maintenance_interval: Duration::from_millis(50),
        },
        &shutdown,
    );

    let admin = ctx::user(ctx::tenant(), USER_GRANTABLE);
    configure_billing(
        &admin,
        &db.app,
        ConfigureBillingCmd {
            currency: Currency::Usd,
            tax_rate: Bps::ZERO,
            expectation: Expectation::Create,
        },
    )
    .await
    .unwrap();
    issue_invoice(
        &admin,
        &db.app,
        &idempotency::IdempotencyKey::parse("wiring-1").unwrap(),
        IssueInvoiceCmd {
            customer_email: CustomerEmail::parse("ap@customer.example").unwrap(),
            lines: vec![NewLine {
                description: LineDescription::parse("Support").unwrap(),
                quantity: Quantity::new(3).unwrap(),
                unit_price_minor: 500,
            }],
        },
    )
    .await
    .unwrap();

    let mut stats = Vec::new();
    for _ in 0..200 {
        stats = get_billing_stats(&admin, &db.app).await.unwrap();
        if !stats.is_empty() {
            break;
        }
        tokio::time::sleep(Duration::from_millis(25)).await;
    }
    shutdown.cancel();
    while let Some(finished) = tasks.join_next().await {
        finished.unwrap().unwrap();
    }
    js.delete_stream(&topology.stream).await.unwrap();

    assert_eq!(
        stats.len(),
        1,
        "the projection was updated through the real wiring"
    );
    assert_eq!(stats[0].issued_total_minor, 1_500);
}
