//! Domain events published by billing (public contract, MSG-003).
//!
//! Rules: versioned and immutable once published (a breaking change is a new
//! `...v2` type); no personal data (events fan out widely and are retained by
//! the broker); amounts in minor units with an explicit currency.

use billing_engine::money::Currency;
use chrono::{DateTime, Utc};
use datastore::DomainEvent;
use serde::{Deserialize, Serialize};
use uuid::Uuid;

use crate::InvoiceId;

/// An invoice was issued.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct InvoiceIssuedV1 {
    pub invoice_id: InvoiceId,
    pub currency: Currency,
    pub total_minor: i64,
    pub issued_at: DateTime<Utc>,
}

impl DomainEvent for InvoiceIssuedV1 {
    const EVENT_TYPE: &'static str = "billing.invoice.issued.v1";
    const AGGREGATE_TYPE: &'static str = "billing.invoice";

    fn aggregate_id(&self) -> Uuid {
        self.invoice_id.as_uuid()
    }
}

/// An issued invoice was voided.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct InvoiceVoidedV1 {
    pub invoice_id: InvoiceId,
    pub currency: Currency,
    pub total_minor: i64,
    pub voided_at: DateTime<Utc>,
}

impl DomainEvent for InvoiceVoidedV1 {
    const EVENT_TYPE: &'static str = "billing.invoice.voided.v1";
    const AGGREGATE_TYPE: &'static str = "billing.invoice";

    fn aggregate_id(&self) -> Uuid {
        self.invoice_id.as_uuid()
    }
}
