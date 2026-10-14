//! HOP 1 (event form): decodes the event and calls the workflow.

use datastore::{DomainEvent as _, TenantTx};
use messaging::{EventHandler, HandlerError, InboundEvent};
use op_context::{OpCtx, Permission};

use super::{StatsChange, record_invoice_stats};
use crate::events::{InvoiceIssuedV1, InvoiceVoidedV1};
use crate::permissions;

/// Durable consumer maintaining invoice statistics. Registered by the worker.
#[derive(Debug, Clone, Copy, Default)]
pub struct RecordInvoiceStats;

#[derive(Debug, thiserror::Error)]
#[error("unexpected event type `{0}`")]
struct UnexpectedEvent(String);

impl EventHandler for RecordInvoiceStats {
    const CONSUMER: &'static str = "billing-record-invoice-stats";
    const EVENT_TYPES: &'static [&'static str] =
        &[InvoiceIssuedV1::EVENT_TYPE, InvoiceVoidedV1::EVENT_TYPE];
    const PERMISSIONS: &'static [Permission] = &[permissions::STATS_RECORD];

    async fn handle(
        &self,
        ctx: &OpCtx,
        tx: &mut TenantTx,
        event: &InboundEvent,
    ) -> Result<(), HandlerError> {
        let change = match event.event_type.as_str() {
            InvoiceIssuedV1::EVENT_TYPE => {
                let issued: InvoiceIssuedV1 = event.decode()?;
                StatsChange::Issued {
                    currency: issued.currency,
                    total_minor: issued.total_minor,
                }
            }
            InvoiceVoidedV1::EVENT_TYPE => {
                let voided: InvoiceVoidedV1 = event.decode()?;
                StatsChange::Voided {
                    currency: voided.currency,
                    total_minor: voided.total_minor,
                }
            }
            other => return Err(HandlerError::permanent(UnexpectedEvent(other.to_owned()))),
        };
        Ok(record_invoice_stats(ctx, tx, change).await?)
    }
}
