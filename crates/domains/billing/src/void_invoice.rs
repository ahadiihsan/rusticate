//! `POST /v1/invoices/{invoice_id}/void`: void an issued invoice.
//!
//! Optimistic concurrency (DB-008): the client must send `If-Match` with the
//! version it saw; the UPDATE is guarded by that version, so of two concurrent
//! voids exactly one succeeds and the other gets 412. No row locks needed.

mod db;
pub(crate) mod http;

use billing_engine::invoice::InvoiceAction;
use datastore::DataStore;
use op_context::OpCtx;

use crate::events::InvoiceVoidedV1;
use crate::get_invoice;
use crate::types::{parse_currency, parse_status};
use crate::{BillingError, InvoiceId, InvoiceView, VoidReason, permissions};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct VoidInvoiceCmd {
    pub invoice_id: InvoiceId,
    pub expected_version: i32,
    pub reason: VoidReason,
}

/// HOP 2: load → engine decides the transition → guarded update → event.
#[tracing::instrument(
    name = "billing.void_invoice",
    skip_all,
    fields(tenant_id = %ctx.tenant_id(), actor_id = %ctx.actor_id(), invoice_id = %cmd.invoice_id)
)]
pub async fn void_invoice(
    ctx: &OpCtx,
    db: &DataStore,
    cmd: VoidInvoiceCmd,
) -> Result<InvoiceView, BillingError> {
    ctx.require(permissions::INVOICE_VOID)?;

    let mut tx = db.begin(ctx).await?;
    let current = db::fetch_target(&mut tx, cmd.invoice_id)
        .await?
        .ok_or(BillingError::InvoiceNotFound)?;
    if current.version != cmd.expected_version {
        return Err(BillingError::StaleVersion);
    }

    // The engine owns the lifecycle; the workflow never decides it inline.
    let next = parse_status(&current.status)?.apply(InvoiceAction::Void)?;

    let voided_at = db::apply_void(&mut tx, &cmd, next.as_str())
        .await?
        .ok_or(BillingError::StaleVersion)?; // lost a race since the read

    tx.emit(&InvoiceVoidedV1 {
        invoice_id: cmd.invoice_id,
        currency: parse_currency(&current.currency)?,
        total_minor: current.total_minor,
        voided_at,
    })
    .await?;

    let invoice = get_invoice::load_in_tx(&mut tx, cmd.invoice_id)
        .await?
        .ok_or(BillingError::InvoiceNotFound)?;
    tx.commit().await?;

    tracing::info!(version = invoice.version, "invoice voided");
    Ok(invoice)
}
