//! Invoice totals and lifecycle.
//!
//! Tax policy: tax is computed once on the subtotal and rounded once
//! (half-to-even). Rounding per line would let rounding error grow with the
//! number of lines.

use std::fmt;
use std::str::FromStr;

use serde::{Deserialize, Serialize};

use crate::EngineError;
use crate::money::{Bps, Currency, Money};

/// Most lines one invoice may carry (bounds work per request).
pub const MAX_LINES: usize = 200;
/// Largest quantity on one line.
pub const MAX_QUANTITY: u32 = 1_000_000;

/// The numeric facts of one invoice line.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct LineInput {
    pub quantity: Quantity,
    pub unit_price: Money,
}

/// A line quantity: `1..=MAX_QUANTITY`. Serializable (commands are
/// fingerprinted) but deliberately not deserializable: construct via [`Quantity::new`].
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize)]
#[serde(transparent)]
pub struct Quantity(u32);

#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
#[error("quantity must be between 1 and {MAX_QUANTITY}")]
pub struct InvalidQuantity;

impl Quantity {
    pub fn new(value: u32) -> Result<Self, InvalidQuantity> {
        if (1..=MAX_QUANTITY).contains(&value) {
            Ok(Self(value))
        } else {
            Err(InvalidQuantity)
        }
    }

    pub const fn get(self) -> u32 {
        self.0
    }
}

/// Computed totals, in the invoice currency.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct InvoiceTotals {
    /// `quantity × unit_price` per line, in input order.
    pub line_totals: Vec<Money>,
    pub subtotal: Money,
    pub tax_rate: Bps,
    pub tax: Money,
    pub total: Money,
}

/// Computes invoice totals.
pub fn compute_totals(
    currency: Currency,
    lines: &[LineInput],
    tax_rate: Bps,
) -> Result<InvoiceTotals, EngineError> {
    if lines.is_empty() {
        return Err(EngineError::NoLines);
    }
    if lines.len() > MAX_LINES {
        return Err(EngineError::TooManyLines { max: MAX_LINES });
    }

    let mut line_totals = Vec::with_capacity(lines.len());
    let mut subtotal = Money::zero(currency);
    for (index, line) in lines.iter().enumerate() {
        if line.unit_price.currency() != currency {
            return Err(EngineError::CurrencyMismatch {
                line: index,
                expected: currency,
                found: line.unit_price.currency(),
            });
        }
        if line.unit_price.is_negative() {
            return Err(EngineError::NegativePrice { line: index });
        }
        let line_total = line.unit_price.checked_mul(line.quantity.get())?;
        subtotal = subtotal.checked_add(line_total)?;
        line_totals.push(line_total);
    }

    let tax = subtotal.apply_rate(tax_rate)?;
    let total = subtotal.checked_add(tax)?;
    Ok(InvoiceTotals {
        line_totals,
        subtotal,
        tax_rate,
        tax,
        total,
    })
}

/// Invoice lifecycle state.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum InvoiceStatus {
    Issued,
    Paid,
    Void,
}

/// A requested lifecycle change.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum InvoiceAction {
    MarkPaid,
    Void,
}

impl InvoiceStatus {
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Issued => "issued",
            Self::Paid => "paid",
            Self::Void => "void",
        }
    }

    /// The state after `action`, or why it is not allowed.
    ///
    /// Exhaustive on purpose: adding a state or action is a compile error
    /// here until its transitions are decided.
    pub fn apply(self, action: InvoiceAction) -> Result<Self, EngineError> {
        match (self, action) {
            (Self::Issued, InvoiceAction::MarkPaid) => Ok(Self::Paid),
            (Self::Issued, InvoiceAction::Void) => Ok(Self::Void),
            (Self::Paid | Self::Void, InvoiceAction::MarkPaid | InvoiceAction::Void) => {
                Err(EngineError::InvalidTransition { from: self, action })
            }
        }
    }
}

impl fmt::Display for InvoiceStatus {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

impl fmt::Display for InvoiceAction {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(match self {
            Self::MarkPaid => "mark paid",
            Self::Void => "void",
        })
    }
}

#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
#[error("unknown invoice status `{0}`")]
pub struct UnknownStatus(pub String);

impl FromStr for InvoiceStatus {
    type Err = UnknownStatus;

    fn from_str(value: &str) -> Result<Self, Self::Err> {
        match value {
            "issued" => Ok(Self::Issued),
            "paid" => Ok(Self::Paid),
            "void" => Ok(Self::Void),
            other => Err(UnknownStatus(other.to_owned())),
        }
    }
}

#[cfg(test)]
#[expect(
    clippy::arithmetic_side_effects,
    reason = "oracles compute exact values in i128/Decimal, which cannot overflow for these inputs"
)]
mod tests {
    use proptest::prelude::*;
    use rust_decimal::prelude::*;

    use super::*;
    use crate::money::{MAX_MINOR, MoneyError};

    fn idr(minor: i64) -> Money {
        Money::from_minor(minor, Currency::Idr).unwrap()
    }

    fn line(quantity: u32, unit_minor: i64) -> LineInput {
        LineInput {
            quantity: Quantity::new(quantity).unwrap(),
            unit_price: idr(unit_minor),
        }
    }

    // ---- Examples ----------------------------------------------------------

    #[test]
    fn computes_subtotal_tax_and_total() {
        let lines = [line(2, 15_000_000), line(1, 4_999)];

        let totals = compute_totals(Currency::Idr, &lines, Bps::new(1_100).unwrap()).unwrap();

        assert_eq!(totals.line_totals, vec![idr(30_000_000), idr(4_999)]);
        assert_eq!(totals.subtotal, idr(30_004_999));
        assert_eq!(totals.tax, idr(3_300_550)); // 3_300_549.89 → 3_300_550
        assert_eq!(totals.total, idr(33_305_549));
    }

    #[test]
    fn rejects_invalid_invoices() {
        let usd_line = LineInput {
            quantity: Quantity::new(1).unwrap(),
            unit_price: Money::from_minor(1, Currency::Usd).unwrap(),
        };
        let too_many = vec![line(1, 1); MAX_LINES + 1];

        assert_eq!(
            compute_totals(Currency::Idr, &[], Bps::ZERO),
            Err(EngineError::NoLines)
        );
        assert!(matches!(
            compute_totals(Currency::Idr, &[usd_line], Bps::ZERO),
            Err(EngineError::CurrencyMismatch { line: 0, .. })
        ));
        assert!(matches!(
            compute_totals(Currency::Idr, &[line(1, -1)], Bps::ZERO),
            Err(EngineError::NegativePrice { line: 0 })
        ));
        assert_eq!(
            compute_totals(Currency::Idr, &too_many, Bps::ZERO),
            Err(EngineError::TooManyLines { max: MAX_LINES })
        );
    }

    #[test]
    fn overflow_is_an_error_not_a_panic() {
        let result = compute_totals(Currency::Idr, &[line(MAX_QUANTITY, MAX_MINOR)], Bps::ZERO);

        assert_eq!(result, Err(EngineError::Money(MoneyError::OutOfRange)));
    }

    #[test]
    fn lifecycle_transitions() {
        use InvoiceAction::{MarkPaid, Void};
        use InvoiceStatus::{Issued, Paid};

        assert_eq!(Issued.apply(Void), Ok(InvoiceStatus::Void));
        assert_eq!(Issued.apply(MarkPaid), Ok(Paid));
        for (from, action) in [
            (Paid, Void),
            (Paid, MarkPaid),
            (InvoiceStatus::Void, Void),
            (InvoiceStatus::Void, MarkPaid),
        ] {
            assert_eq!(
                from.apply(action),
                Err(EngineError::InvalidTransition { from, action })
            );
        }
    }

    #[test]
    fn status_strings_round_trip() {
        for status in [
            InvoiceStatus::Issued,
            InvoiceStatus::Paid,
            InvoiceStatus::Void,
        ] {
            assert_eq!(status.as_str().parse::<InvoiceStatus>(), Ok(status));
        }
    }

    // ---- Properties (ENG-005) ----------------------------------------------
    //
    // Each property checks the engine against an INDEPENDENT oracle or an
    // algebraic law, never against a re-statement of the implementation.

    fn lines_strategy() -> impl Strategy<Value = Vec<LineInput>> {
        prop::collection::vec((1..=1_000_u32, 0..=1_000_000_000_i64), 1..=20)
            .prop_map(|raw| raw.into_iter().map(|(q, p)| line(q, p)).collect())
    }

    proptest! {
        /// Oracle: rust_decimal's MidpointNearestEven on the exact product.
        #[test]
        fn tax_matches_decimal_half_even_oracle(subtotal in 0..=MAX_MINOR, bps in 0..=Bps::MAX_VALUE) {
            let engine = idr(subtotal).apply_rate(Bps::new(bps).unwrap()).unwrap().minor();

            let exact = Decimal::from(subtotal) * Decimal::from(bps) / Decimal::from(10_000);
            let oracle = exact
                .round_dp_with_strategy(0, RoundingStrategy::MidpointNearestEven)
                .to_i64()
                .unwrap();
            prop_assert_eq!(engine, oracle);
        }

        /// Laws: subtotal is the exact (i128) sum of line totals; total is
        /// subtotal + tax; 0 ≤ tax ≤ subtotal; line order is irrelevant.
        #[test]
        fn totals_obey_conservation_bounds_and_order_independence(
            lines in lines_strategy(),
            bps in 0..=Bps::MAX_VALUE,
        ) {
            let rate = Bps::new(bps).unwrap();
            let totals = compute_totals(Currency::Idr, &lines, rate).unwrap();

            let exact_sum: i128 = lines
                .iter()
                .map(|l| i128::from(l.unit_price.minor()) * i128::from(l.quantity.get()))
                .sum();
            prop_assert_eq!(i128::from(totals.subtotal.minor()), exact_sum);
            prop_assert_eq!(totals.total.minor(), totals.subtotal.minor() + totals.tax.minor());
            prop_assert!(totals.tax.minor() >= 0 && totals.tax.minor() <= totals.subtotal.minor());

            let mut reversed = lines.clone();
            reversed.reverse();
            let reversed_totals = compute_totals(Currency::Idr, &reversed, rate).unwrap();
            prop_assert_eq!(reversed_totals.total, totals.total);
        }

        /// Totality: extreme inputs yield Ok or a typed error, never a panic.
        #[test]
        fn never_panics(quantity in 1..=MAX_QUANTITY, unit in 0..=MAX_MINOR, bps in 0..=Bps::MAX_VALUE) {
            let result = compute_totals(Currency::Idr, &[line(quantity, unit)], Bps::new(bps).unwrap());
            prop_assert!(matches!(result, Ok(_) | Err(EngineError::Money(MoneyError::OutOfRange))));
        }
    }
}
