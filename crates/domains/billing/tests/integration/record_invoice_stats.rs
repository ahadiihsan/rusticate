use billing::events::{InvoiceIssuedV1, InvoiceVoidedV1};
use billing::get_billing_stats::get_billing_stats;
use billing::record_invoice_stats::{StatsChange, record_invoice_stats};
use billing::{BillingError, RecordInvoiceStats};
use billing_engine::money::Currency;
use chrono::Utc;
use datastore::DomainEvent;
use messaging::{InboundEvent, Outcome, process};
use op_context::{CorrelationId, TenantId};
use serde::Serialize;
use sqlx::postgres::{PgConnectOptions, PgPoolOptions};
use test_kit::ctx;
use uuid::Uuid;

use crate::support;

fn deliver<E: DomainEvent + Serialize>(tenant: TenantId, event: &E) -> InboundEvent {
    InboundEvent {
        id: Uuid::now_v7(),
        event_type: E::EVENT_TYPE.to_owned(),
        tenant_id: tenant,
        correlation_id: CorrelationId::new_v7(),
        aggregate_id: event.aggregate_id(),
        traceparent: None,
        payload: serde_json::to_value(event).unwrap(),
    }
}

#[sqlx::test(migrations = false)]
async fn projects_issued_and_voided_totals_exactly_once(
    pool: PgPoolOptions,
    conn: PgConnectOptions,
) {
    let db = support::db(pool, conn).await;
    let tenant = ctx::tenant();
    let invoice_id = billing::InvoiceId::new();
    let issued = deliver(
        tenant,
        &InvoiceIssuedV1 {
            invoice_id,
            currency: Currency::Idr,
            total_minor: 5_000,
            issued_at: Utc::now(),
        },
    );
    let voided = deliver(
        tenant,
        &InvoiceVoidedV1 {
            invoice_id,
            currency: Currency::Idr,
            total_minor: 5_000,
            voided_at: Utc::now(),
        },
    );

    assert_eq!(
        process(&db.app, &RecordInvoiceStats, &issued)
            .await
            .unwrap(),
        Outcome::Processed
    );
    assert_eq!(
        process(&db.app, &RecordInvoiceStats, &issued)
            .await
            .unwrap(),
        Outcome::Duplicate
    );
    assert_eq!(
        process(&db.app, &RecordInvoiceStats, &voided)
            .await
            .unwrap(),
        Outcome::Processed
    );

    let stats = get_billing_stats(&support::admin(tenant), &db.app)
        .await
        .unwrap();
    assert_eq!(stats.len(), 1);
    assert_eq!(
        (
            stats[0].issued_count,
            stats[0].issued_total_minor,
            stats[0].voided_count,
            stats[0].voided_total_minor
        ),
        (1, 5_000, 1, 5_000)
    );
}

#[sqlx::test(migrations = false)]
async fn malformed_payloads_fail_permanently(pool: PgPoolOptions, conn: PgConnectOptions) {
    let db = support::db(pool, conn).await;
    let mut event = deliver(
        ctx::tenant(),
        &InvoiceIssuedV1 {
            invoice_id: billing::InvoiceId::new(),
            currency: Currency::Idr,
            total_minor: 1,
            issued_at: Utc::now(),
        },
    );
    event.payload = serde_json::json!({ "unexpected": true });

    let result = process(&db.app, &RecordInvoiceStats, &event).await;

    assert!(matches!(result, Err(err) if !err.is_transient()));
}

#[sqlx::test(migrations = false)]
async fn only_the_consumer_identity_may_record_stats(pool: PgPoolOptions, conn: PgConnectOptions) {
    let db = support::db(pool, conn).await;
    let human_admin = support::admin(ctx::tenant());
    let mut tx = db.app.begin(&human_admin).await.unwrap();

    let outcome = record_invoice_stats(
        &human_admin,
        &mut tx,
        StatsChange::Issued {
            currency: Currency::Idr,
            total_minor: 1,
        },
    )
    .await;

    assert!(matches!(outcome, Err(BillingError::Forbidden(_))));
}
