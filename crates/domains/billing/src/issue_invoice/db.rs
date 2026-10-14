//! HOP 3: SQL for `issue_invoice`. Compile-time checked, no business branching.

use billing_engine::invoice::InvoiceTotals;
use billing_engine::money::Money;
use chrono::{DateTime, Utc};
use datastore::TenantTx;

use super::NewLine;
use crate::types::SettingsRow;
use crate::{BillingError, CustomerEmail, InvoiceId};

pub(super) async fn load_settings(tx: &mut TenantTx) -> Result<Option<SettingsRow>, sqlx::Error> {
    sqlx::query_as!(
        SettingsRow,
        r#"
        SELECT currency, tax_rate_bps, version
        FROM billing.settings
        WHERE tenant_id = $1
        "#,
        tx.tenant_id().as_uuid(),
    )
    .fetch_optional(tx.conn())
    .await
}

/// Inserts the invoice header; returns the database-assigned issue time.
pub(super) async fn insert_invoice(
    tx: &mut TenantTx,
    invoice_id: InvoiceId,
    customer_email: &CustomerEmail,
    totals: &InvoiceTotals,
) -> Result<DateTime<Utc>, sqlx::Error> {
    sqlx::query_scalar!(
        r#"
        INSERT INTO billing.invoices (
            tenant_id, id, customer_email, currency, status,
            subtotal_minor, tax_rate_bps, tax_minor, total_minor, issued_by
        )
        VALUES ($1, $2, $3, $4, 'issued', $5, $6, $7, $8, $9)
        RETURNING issued_at
        "#,
        tx.tenant_id().as_uuid(),
        invoice_id.as_uuid(),
        customer_email.expose(),
        totals.subtotal.currency().code(),
        totals.subtotal.minor(),
        i32::from(totals.tax_rate.get()),
        totals.tax.minor(),
        totals.total.minor(),
        tx.actor_id().as_uuid(),
    )
    .fetch_one(tx.conn())
    .await
}

/// Inserts all lines in ONE statement (PERF-003: set-based writes, no N+1).
pub(super) async fn insert_lines(
    tx: &mut TenantTx,
    invoice_id: InvoiceId,
    lines: &[NewLine],
    line_totals: &[Money],
) -> Result<(), BillingError> {
    let overflow = || BillingError::Corrupt("invoice line exceeds column range".to_owned());
    let line_numbers = (0..lines.len())
        .map(|n| i16::try_from(n).map_err(|_| overflow()))
        .collect::<Result<Vec<i16>, _>>()?;
    let quantities = lines
        .iter()
        .map(|line| i32::try_from(line.quantity.get()).map_err(|_| overflow()))
        .collect::<Result<Vec<i32>, _>>()?;
    let descriptions: Vec<&str> = lines.iter().map(|line| line.description.as_str()).collect();
    let unit_prices: Vec<i64> = lines.iter().map(|line| line.unit_price_minor).collect();
    let totals: Vec<i64> = line_totals.iter().map(|total| total.minor()).collect();

    sqlx::query!(
        r#"
        INSERT INTO billing.invoice_lines (
            tenant_id, invoice_id, line_no, description, quantity, unit_price_minor, line_total_minor
        )
        SELECT $1, $2, line.*
        FROM UNNEST($3::int2[], $4::text[], $5::int4[], $6::int8[], $7::int8[])
            AS line(line_no, description, quantity, unit_price_minor, line_total_minor)
        "#,
        tx.tenant_id().as_uuid(),
        invoice_id.as_uuid(),
        &line_numbers,
        &descriptions as &[&str],
        &quantities,
        &unit_prices,
        &totals,
    )
    .execute(tx.conn())
    .await?;
    Ok(())
}
