use api_kit::pagination::{PageParams, PageRequest};
use billing::list_invoices::{ListInvoicesQuery, list_invoices};
use billing::void_invoice::{VoidInvoiceCmd, void_invoice};
use billing::{InvoiceId, VoidReason};
use billing_engine::invoice::InvoiceStatus;
use sqlx::postgres::{PgConnectOptions, PgPoolOptions};
use test_kit::ctx;

use crate::support;

fn query(limit: u32, cursor: Option<String>, status: Option<InvoiceStatus>) -> ListInvoicesQuery {
    ListInvoicesQuery {
        page: PageRequest::try_from(PageParams {
            limit: Some(limit),
            cursor,
        })
        .unwrap(),
        status,
    }
}

#[sqlx::test(migrations = false)]
async fn pages_newest_first_without_gaps_or_repeats(pool: PgPoolOptions, conn: PgConnectOptions) {
    let db = support::db(pool, conn).await;
    let admin = support::admin(ctx::tenant());
    support::configure(&db, &admin).await;
    let mut issued = Vec::new();
    for n in 0..5 {
        issued.push(
            support::issue(&db, &admin, &format!("k{n}"), 1_000)
                .await
                .id,
        );
    }

    let mut seen: Vec<InvoiceId> = Vec::new();
    let mut cursor = None;
    loop {
        let page = list_invoices(&admin, &db.app, query(2, cursor, None))
            .await
            .unwrap();
        seen.extend(page.items.iter().map(|invoice| invoice.id));
        match page.next_cursor {
            Some(next) => cursor = Some(next),
            None => break,
        }
    }

    issued.reverse();
    assert_eq!(seen, issued);
}

#[sqlx::test(migrations = false)]
async fn filters_by_status_and_tenant(pool: PgPoolOptions, conn: PgConnectOptions) {
    let db = support::db(pool, conn).await;
    let admin = support::admin(ctx::tenant());
    support::configure(&db, &admin).await;
    let keep = support::issue(&db, &admin, "k1", 1_000).await;
    let voided = support::issue(&db, &admin, "k2", 1_000).await;
    void_invoice(
        &admin,
        &db.app,
        VoidInvoiceCmd {
            invoice_id: voided.id,
            expected_version: voided.version,
            reason: VoidReason::parse("duplicate").unwrap(),
        },
    )
    .await
    .unwrap();

    let issued_only = list_invoices(
        &admin,
        &db.app,
        query(10, None, Some(InvoiceStatus::Issued)),
    )
    .await
    .unwrap();
    let other_tenant = list_invoices(
        &support::admin(ctx::tenant()),
        &db.app,
        query(10, None, None),
    )
    .await
    .unwrap();

    assert_eq!(
        issued_only.items.iter().map(|i| i.id).collect::<Vec<_>>(),
        [keep.id]
    );
    assert_eq!(other_tenant.items.len(), 0, "another tenant sees nothing");
}
