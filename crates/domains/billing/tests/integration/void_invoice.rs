use billing::void_invoice::{VoidInvoiceCmd, void_invoice};
use billing::{BillingError, InvoiceView, VoidReason};
use billing_engine::EngineError;
use billing_engine::invoice::InvoiceStatus;
use sqlx::postgres::{PgConnectOptions, PgPoolOptions};
use test_kit::ctx;

use crate::support;

fn void(invoice: &InvoiceView, version: i32) -> VoidInvoiceCmd {
    VoidInvoiceCmd {
        invoice_id: invoice.id,
        expected_version: version,
        reason: VoidReason::parse("wrong customer").unwrap(),
    }
}

#[sqlx::test(migrations = false)]
async fn voiding_bumps_the_version_and_emits_an_event(pool: PgPoolOptions, conn: PgConnectOptions) {
    let db = support::db(pool, conn).await;
    let admin = support::admin(ctx::tenant());
    support::configure(&db, &admin).await;
    let issued = support::issue(&db, &admin, "k1", 1_000).await;

    let voided = void_invoice(&admin, &db.app, void(&issued, issued.version))
        .await
        .unwrap();

    assert_eq!(voided.status, InvoiceStatus::Void);
    assert_eq!(voided.version, issued.version + 1);
    assert_eq!(voided.void_reason.as_deref(), Some("wrong customer"));
    assert_eq!(
        support::count(&db, "SELECT count(*) FROM platform.outbox_events WHERE event_type = 'billing.invoice.voided.v1'").await,
        1
    );
}

#[sqlx::test(migrations = false)]
async fn a_stale_version_is_rejected(pool: PgPoolOptions, conn: PgConnectOptions) {
    let db = support::db(pool, conn).await;
    let admin = support::admin(ctx::tenant());
    support::configure(&db, &admin).await;
    let issued = support::issue(&db, &admin, "k1", 1_000).await;

    let stale = void_invoice(&admin, &db.app, void(&issued, issued.version + 7)).await;

    assert!(matches!(stale, Err(BillingError::StaleVersion)));
}

#[sqlx::test(migrations = false)]
async fn concurrent_voids_exactly_one_wins(pool: PgPoolOptions, conn: PgConnectOptions) {
    let db = support::db(pool, conn).await;
    let admin = support::admin(ctx::tenant());
    support::configure(&db, &admin).await;
    let issued = support::issue(&db, &admin, "k1", 1_000).await;

    let (a, b) = tokio::join!(
        void_invoice(&admin, &db.app, void(&issued, issued.version)),
        void_invoice(&admin, &db.app, void(&issued, issued.version)),
    );

    let outcomes = [a.is_ok(), b.is_ok()];
    assert_eq!(
        outcomes.iter().filter(|ok| **ok).count(),
        1,
        "exactly one void succeeds"
    );
    assert_eq!(support::count(&db, "SELECT count(*) FROM platform.outbox_events WHERE event_type = 'billing.invoice.voided.v1'").await, 1);
}

#[sqlx::test(migrations = false)]
async fn a_void_invoice_cannot_be_voided_again(pool: PgPoolOptions, conn: PgConnectOptions) {
    let db = support::db(pool, conn).await;
    let admin = support::admin(ctx::tenant());
    support::configure(&db, &admin).await;
    let issued = support::issue(&db, &admin, "k1", 1_000).await;
    let voided = void_invoice(&admin, &db.app, void(&issued, issued.version))
        .await
        .unwrap();

    let again = void_invoice(&admin, &db.app, void(&voided, voided.version)).await;

    assert!(matches!(
        again,
        Err(BillingError::Rule(EngineError::InvalidTransition { .. }))
    ));
}
