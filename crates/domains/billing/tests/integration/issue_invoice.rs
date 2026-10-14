use billing::BillingError;
use billing::get_invoice::get_invoice;
use billing::issue_invoice::issue_invoice;
use billing::permissions::INVOICE_READ;
use billing_engine::invoice::InvoiceStatus;
use idempotency::IdempotencyError;
use sqlx::postgres::{PgConnectOptions, PgPoolOptions};
use test_kit::ctx;

use crate::support;

#[sqlx::test(migrations = false)]
async fn issues_with_server_side_tax_and_emits_an_event(
    pool: PgPoolOptions,
    conn: PgConnectOptions,
) {
    let db = support::db(pool, conn).await;
    let admin = support::admin(ctx::tenant());
    support::configure(&db, &admin).await;

    let invoice = issue_invoice(
        &admin,
        &db.app,
        &support::key("k1"),
        support::invoice_cmd(15_000_000, 2),
    )
    .await
    .unwrap();

    assert_eq!(invoice.status, InvoiceStatus::Issued);
    assert_eq!(invoice.subtotal_minor, 30_000_000);
    assert_eq!(
        invoice.tax_rate_bps, 1_100,
        "tax rate comes from tenant settings"
    );
    assert_eq!(invoice.tax_minor, 3_300_000);
    assert_eq!(invoice.total_minor, 33_300_000);
    assert_eq!(
        get_invoice(&admin, &db.app, invoice.id).await.unwrap(),
        invoice,
        "stored == returned"
    );
    assert_eq!(
        support::count(&db, "SELECT count(*) FROM platform.outbox_events WHERE event_type = 'billing.invoice.issued.v1'").await,
        1
    );
}

#[sqlx::test(migrations = false)]
async fn retries_with_the_same_key_replay_the_original_invoice(
    pool: PgPoolOptions,
    conn: PgConnectOptions,
) {
    let db = support::db(pool, conn).await;
    let admin = support::admin(ctx::tenant());
    support::configure(&db, &admin).await;

    let first = support::issue(&db, &admin, "same-key", 1_000).await;
    let retry = support::issue(&db, &admin, "same-key", 1_000).await;

    assert_eq!(first, retry);
    assert_eq!(
        support::count(&db, "SELECT count(*) FROM platform.outbox_events").await,
        1,
        "no duplicate side effects"
    );
}

#[sqlx::test(migrations = false)]
async fn reusing_a_key_for_a_different_invoice_is_rejected(
    pool: PgPoolOptions,
    conn: PgConnectOptions,
) {
    let db = support::db(pool, conn).await;
    let admin = support::admin(ctx::tenant());
    support::configure(&db, &admin).await;
    support::issue(&db, &admin, "k1", 1_000).await;

    let reused = issue_invoice(
        &admin,
        &db.app,
        &support::key("k1"),
        support::invoice_cmd(2_000, 1),
    )
    .await;

    assert!(matches!(
        reused,
        Err(BillingError::Idempotency(IdempotencyError::KeyReused))
    ));
}

#[sqlx::test(migrations = false)]
async fn requires_configuration_and_permission(pool: PgPoolOptions, conn: PgConnectOptions) {
    let db = support::db(pool, conn).await;
    let tenant = ctx::tenant();

    let unconfigured = issue_invoice(
        &support::admin(tenant),
        &db.app,
        &support::key("k"),
        support::invoice_cmd(1, 1),
    )
    .await;
    let read_only = issue_invoice(
        &ctx::user(tenant, &[INVOICE_READ]),
        &db.app,
        &support::key("k"),
        support::invoice_cmd(1, 1),
    )
    .await;

    assert!(matches!(unconfigured, Err(BillingError::NotConfigured)));
    assert!(matches!(read_only, Err(BillingError::Forbidden(_))));
}

#[sqlx::test(migrations = false)]
async fn another_tenant_cannot_see_the_invoice(pool: PgPoolOptions, conn: PgConnectOptions) {
    let db = support::db(pool, conn).await;
    let owner = support::admin(ctx::tenant());
    support::configure(&db, &owner).await;
    let invoice = support::issue(&db, &owner, "k1", 1_000).await;

    let intruder = support::admin(ctx::tenant());
    let attempt = get_invoice(&intruder, &db.app, invoice.id).await;

    assert!(matches!(attempt, Err(BillingError::InvoiceNotFound)));
}
