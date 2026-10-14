//! Response DTOs shared by several billing endpoints. Feature-specific
//! request/response types live in the feature's `http.rs`.

use api_kit::etag;
use axum::http::{HeaderName, HeaderValue, header};
use chrono::{DateTime, Utc};
use serde::Serialize;
use utoipa::ToSchema;
use uuid::Uuid;

use crate::{BillingSettings, InvoiceLineView, InvoiceSummary, InvoiceView};

/// Tenant billing settings.
#[derive(Debug, Serialize, ToSchema)]
pub(crate) struct BillingSettingsResponse {
    /// ISO 4217 currency of every invoice issued by this tenant.
    #[schema(example = "IDR")]
    pub(crate) currency: String,
    /// Tax rate in basis points (1100 = 11 %).
    #[schema(example = 1100)]
    pub(crate) tax_rate_bps: u16,
    /// Also sent as the `ETag` header; send it back in `If-Match` to update.
    pub(crate) version: i32,
}

impl From<BillingSettings> for BillingSettingsResponse {
    fn from(settings: BillingSettings) -> Self {
        Self {
            currency: settings.currency.code().to_owned(),
            tax_rate_bps: settings.tax_rate.get(),
            version: settings.version,
        }
    }
}

/// One invoice line.
#[derive(Debug, Serialize, ToSchema)]
pub(crate) struct InvoiceLineResponse {
    pub(crate) description: String,
    pub(crate) quantity: u32,
    /// Unit price in minor units of the invoice currency.
    pub(crate) unit_price_minor: i64,
    pub(crate) line_total_minor: i64,
}

/// A complete invoice. Amounts are integers in minor units of `currency`.
#[derive(Debug, Serialize, ToSchema)]
pub(crate) struct InvoiceResponse {
    pub(crate) id: Uuid,
    #[schema(example = "issued")]
    pub(crate) status: String,
    pub(crate) customer_email: String,
    #[schema(example = "IDR")]
    pub(crate) currency: String,
    pub(crate) lines: Vec<InvoiceLineResponse>,
    pub(crate) subtotal_minor: i64,
    pub(crate) tax_rate_bps: u16,
    pub(crate) tax_minor: i64,
    pub(crate) total_minor: i64,
    /// Also sent as the `ETag` header.
    pub(crate) version: i32,
    pub(crate) issued_at: DateTime<Utc>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(crate) voided_at: Option<DateTime<Utc>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(crate) void_reason: Option<String>,
}

impl From<InvoiceView> for InvoiceResponse {
    fn from(invoice: InvoiceView) -> Self {
        Self {
            id: invoice.id.as_uuid(),
            status: invoice.status.as_str().to_owned(),
            customer_email: invoice.customer_email.expose().to_owned(),
            currency: invoice.currency.code().to_owned(),
            lines: invoice
                .lines
                .into_iter()
                .map(InvoiceLineResponse::from)
                .collect(),
            subtotal_minor: invoice.subtotal_minor,
            tax_rate_bps: invoice.tax_rate_bps,
            tax_minor: invoice.tax_minor,
            total_minor: invoice.total_minor,
            version: invoice.version,
            issued_at: invoice.issued_at,
            voided_at: invoice.voided_at,
            void_reason: invoice.void_reason,
        }
    }
}

impl From<InvoiceLineView> for InvoiceLineResponse {
    fn from(line: InvoiceLineView) -> Self {
        Self {
            description: line.description,
            quantity: line.quantity,
            unit_price_minor: line.unit_price_minor,
            line_total_minor: line.line_total_minor,
        }
    }
}

/// An invoice in a list.
#[derive(Debug, Serialize, ToSchema)]
pub(crate) struct InvoiceSummaryResponse {
    pub(crate) id: Uuid,
    pub(crate) status: String,
    pub(crate) customer_email: String,
    pub(crate) currency: String,
    pub(crate) total_minor: i64,
    pub(crate) version: i32,
    pub(crate) issued_at: DateTime<Utc>,
}

impl From<InvoiceSummary> for InvoiceSummaryResponse {
    fn from(invoice: InvoiceSummary) -> Self {
        Self {
            id: invoice.id.as_uuid(),
            status: invoice.status.as_str().to_owned(),
            customer_email: invoice.customer_email.expose().to_owned(),
            currency: invoice.currency.code().to_owned(),
            total_minor: invoice.total_minor,
            version: invoice.version,
            issued_at: invoice.issued_at,
        }
    }
}

/// `ETag` header for a versioned resource.
pub(crate) fn etag_header(version: i32) -> [(HeaderName, HeaderValue); 1] {
    [(header::ETAG, etag(version))]
}
