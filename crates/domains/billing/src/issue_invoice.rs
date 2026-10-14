//! `POST /v1/invoices`: issue an invoice. **The canonical reference workflow.**
//!
//! Requires `Idempotency-Key`. Currency and tax rate come from the tenant's
//! billing settings, never from the client. A replay returns the invoice as
//! first issued (even if it was voided since); `GET` its `Location` for the
//! current state.

mod db;
pub(crate) mod http;

use billing_engine::invoice::{InvoiceStatus, LineInput, Quantity, compute_totals};
use billing_engine::money::Money;
use datastore::DataStore;
use idempotency::{Claim, Fingerprint, IdempotencyKey};
use op_context::OpCtx;
use serde::Serialize;

use crate::events::InvoiceIssuedV1;
use crate::{
    BillingError, BillingSettings, CustomerEmail, InvoiceId, InvoiceLineView, InvoiceView,
    LineDescription, permissions,
};

/// Idempotency operation name (scopes keys to this workflow).
pub const OPERATION: &str = "billing.issue_invoice";

/// A validated request to issue an invoice. Serializable: its canonical JSON
/// is the idempotency fingerprint.
#[derive(Debug, Clone, Serialize)]
pub struct IssueInvoiceCmd {
    pub customer_email: CustomerEmail,
    pub lines: Vec<NewLine>,
}

/// A validated invoice line. Prices are minor units of the tenant's currency.
#[derive(Debug, Clone, Serialize)]
pub struct NewLine {
    pub description: LineDescription,
    pub quantity: Quantity,
    pub unit_price_minor: i64,
}

/// HOP 2: the workflow orchestrator.
///
/// authorize → open tenant tx → claim idempotency key → load settings →
/// pure engine → persist (set-based) → outbox event → complete key → commit.
#[tracing::instrument(
    name = "billing.issue_invoice",
    skip_all,
    fields(
        tenant_id = %ctx.tenant_id(),
        actor_id = %ctx.actor_id(),
        line_count = cmd.lines.len(),
        invoice_id = tracing::field::Empty,
    )
)]
pub async fn issue_invoice(
    ctx: &OpCtx,
    db: &DataStore,
    key: &IdempotencyKey,
    cmd: IssueInvoiceCmd,
) -> Result<InvoiceView, BillingError> {
    // 1. Capability check: always first.
    ctx.require(permissions::INVOICE_ISSUE)?;
    let fingerprint = Fingerprint::of(OPERATION, &cmd)?;

    // 2. One tenant-scoped transaction for everything below.
    let mut tx = db.begin(ctx).await?;

    // 3. Idempotency: a retry returns the original result, unchanged.
    let pending = match idempotency::claim(&mut tx, OPERATION, key, &fingerprint).await? {
        Claim::Replay(previous) => return Ok(previous),
        Claim::Fresh(pending) => pending,
    };

    // 4. Server-side facts the client must not choose.
    let settings: BillingSettings = db::load_settings(&mut tx)
        .await?
        .ok_or(BillingError::NotConfigured)?
        .try_into()?;

    // 5. Pure business rules (no I/O, exhaustively tested in billing-engine).
    let inputs = cmd
        .lines
        .iter()
        .map(|line| {
            Ok(LineInput {
                quantity: line.quantity,
                unit_price: Money::from_minor(line.unit_price_minor, settings.currency)?,
            })
        })
        .collect::<Result<Vec<_>, BillingError>>()?;
    let totals = compute_totals(settings.currency, &inputs, settings.tax_rate)?;

    // 6. Persist (set-based: one statement for all lines).
    let invoice_id = InvoiceId::new();
    tracing::Span::current().record("invoice_id", tracing::field::display(invoice_id));
    let issued_at = db::insert_invoice(&mut tx, invoice_id, &cmd.customer_email, &totals).await?;
    db::insert_lines(&mut tx, invoice_id, &cmd.lines, &totals.line_totals).await?;

    // 7. Domain event, atomically with the state change (never a dual write).
    tx.emit(&InvoiceIssuedV1 {
        invoice_id,
        currency: settings.currency,
        total_minor: totals.total.minor(),
        issued_at,
    })
    .await?;

    let invoice = InvoiceView {
        id: invoice_id,
        status: InvoiceStatus::Issued,
        customer_email: cmd.customer_email,
        currency: settings.currency,
        lines: cmd
            .lines
            .into_iter()
            .zip(&totals.line_totals)
            .map(|(line, total)| InvoiceLineView {
                description: line.description.as_str().to_owned(),
                quantity: line.quantity.get(),
                unit_price_minor: line.unit_price_minor,
                line_total_minor: total.minor(),
            })
            .collect(),
        subtotal_minor: totals.subtotal.minor(),
        tax_rate_bps: totals.tax_rate.get(),
        tax_minor: totals.tax.minor(),
        total_minor: totals.total.minor(),
        version: 1,
        issued_at,
        voided_at: None,
        void_reason: None,
    };

    // 8. Store the result for replays, then commit everything at once.
    pending.complete(&mut tx, &invoice).await?;
    tx.commit().await?;

    tracing::info!(total_minor = invoice.total_minor, "invoice issued");
    Ok(invoice)
}
