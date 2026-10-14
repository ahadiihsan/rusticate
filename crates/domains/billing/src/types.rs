//! Value objects and read models of the billing context (parse, don't validate).

use std::fmt;

use billing_engine::invoice::InvoiceStatus;
use billing_engine::money::{Bps, Currency};
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use telemetry::Sensitive;
use uuid::Uuid;

use crate::BillingError;

/// Invoice identifier (UUIDv7).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(transparent)]
pub struct InvoiceId(Uuid);

impl InvoiceId {
    pub fn new() -> Self {
        Self(Uuid::now_v7())
    }

    pub const fn from_uuid(id: Uuid) -> Self {
        Self(id)
    }

    pub const fn as_uuid(self) -> Uuid {
        self.0
    }
}

impl Default for InvoiceId {
    fn default() -> Self {
        Self::new()
    }
}

impl fmt::Display for InvoiceId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        fmt::Display::fmt(&self.0, f)
    }
}

/// A syntactically valid email address. Personal data: `Debug` is redacted.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(transparent)]
pub struct CustomerEmail(Sensitive<String>);

impl CustomerEmail {
    const MAX_LEN: usize = 254;

    /// Deliberately conservative: one `@`, non-empty local part, a dotted
    /// domain, no whitespace or control characters. Deliverability is not
    /// checked here.
    pub fn parse(raw: &str) -> Result<Self, &'static str> {
        let value = raw.trim();
        let Some((local, domain)) = value.split_once('@') else {
            return Err("must contain `@`");
        };
        let well_formed = !local.is_empty()
            && domain.contains('.')
            && !domain.starts_with('.')
            && !domain.ends_with('.')
            && !domain.contains('@')
            && value.len() <= Self::MAX_LEN
            && !value.chars().any(|c| c.is_whitespace() || c.is_control());
        if well_formed {
            Ok(Self(Sensitive::new(value.to_owned())))
        } else {
            Err("is not a valid email address")
        }
    }

    pub fn expose(&self) -> &str {
        self.0.expose()
    }
}

macro_rules! bounded_text {
    ($(#[$meta:meta])* $name:ident, $max:expr) => {
        $(#[$meta])*
        #[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
        #[serde(transparent)]
        pub struct $name(String);

        impl $name {
            pub const MAX_CHARS: usize = $max;

            /// Trimmed, `1..=MAX_CHARS` characters, no control characters.
            pub fn parse(raw: &str) -> Result<Self, &'static str> {
                let value = raw.trim();
                let length = value.chars().count();
                if length == 0 {
                    Err("must not be blank")
                } else if length > Self::MAX_CHARS {
                    Err(concat!("must be at most ", stringify!($max), " characters"))
                } else if value.chars().any(char::is_control) {
                    Err("must not contain control characters")
                } else {
                    Ok(Self(value.to_owned()))
                }
            }

            pub fn as_str(&self) -> &str {
                &self.0
            }
        }
    };
}

bounded_text!(
    /// Free-text description of an invoice line.
    LineDescription,
    500
);

bounded_text!(
    /// Why an invoice was voided (audit trail).
    VoidReason,
    500
);

/// Tenant billing configuration.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct BillingSettings {
    pub currency: Currency,
    pub tax_rate: Bps,
    pub version: i32,
}

/// Raw `billing.settings` row, shared by the features that read it.
#[derive(Debug)]
pub(crate) struct SettingsRow {
    pub(crate) currency: String,
    pub(crate) tax_rate_bps: i32,
    pub(crate) version: i32,
}

impl TryFrom<SettingsRow> for BillingSettings {
    type Error = BillingError;

    fn try_from(row: SettingsRow) -> Result<Self, Self::Error> {
        Ok(Self {
            currency: parse_currency(&row.currency)?,
            tax_rate: u16::try_from(row.tax_rate_bps)
                .ok()
                .and_then(|bps| Bps::new(bps).ok())
                .ok_or_else(|| BillingError::Corrupt(format!("tax rate {}", row.tax_rate_bps)))?,
            version: row.version,
        })
    }
}

/// One invoice line as stored.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct InvoiceLineView {
    pub description: String,
    pub quantity: u32,
    pub unit_price_minor: i64,
    pub line_total_minor: i64,
}

/// A complete invoice. Also the stored result of an idempotent issue.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct InvoiceView {
    pub id: InvoiceId,
    pub status: InvoiceStatus,
    pub customer_email: CustomerEmail,
    pub currency: Currency,
    pub lines: Vec<InvoiceLineView>,
    pub subtotal_minor: i64,
    pub tax_rate_bps: u16,
    pub tax_minor: i64,
    pub total_minor: i64,
    pub version: i32,
    pub issued_at: DateTime<Utc>,
    pub voided_at: Option<DateTime<Utc>>,
    pub void_reason: Option<String>,
}

/// An invoice in a list (no lines).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct InvoiceSummary {
    pub id: InvoiceId,
    pub status: InvoiceStatus,
    pub customer_email: CustomerEmail,
    pub currency: Currency,
    pub total_minor: i64,
    pub version: i32,
    pub issued_at: DateTime<Utc>,
}

/// Issued/voided aggregates for one currency (eventually consistent).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct CurrencyStats {
    pub currency: Currency,
    pub issued_count: i64,
    pub issued_total_minor: i64,
    pub voided_count: i64,
    pub voided_total_minor: i64,
}

pub(crate) fn parse_currency(code: &str) -> Result<Currency, BillingError> {
    code.parse()
        .map_err(|_| BillingError::Corrupt(format!("unknown currency `{code}`")))
}

pub(crate) fn parse_status(status: &str) -> Result<InvoiceStatus, BillingError> {
    status
        .parse()
        .map_err(|_| BillingError::Corrupt(format!("unknown invoice status `{status}`")))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn email_parsing() {
        assert!(CustomerEmail::parse(" ana@example.co.id ").is_ok());
        for bad in [
            "",
            "ana",
            "@example.com",
            "ana@example",
            "ana@.com",
            "ana@ex ample.com",
            "a@b@c.com",
        ] {
            assert!(CustomerEmail::parse(bad).is_err(), "accepted {bad:?}");
        }
    }

    #[test]
    fn email_debug_is_redacted() {
        let email = CustomerEmail::parse("ana@example.com").unwrap();

        assert!(!format!("{email:?}").contains("ana"));
    }

    #[test]
    fn bounded_text_rules() {
        assert_eq!(
            LineDescription::parse("  Consulting  ").unwrap().as_str(),
            "Consulting"
        );
        assert!(LineDescription::parse("   ").is_err());
        assert!(LineDescription::parse(&"x".repeat(501)).is_err());
        assert!(VoidReason::parse("bad\u{0007}bell").is_err());
    }
}
