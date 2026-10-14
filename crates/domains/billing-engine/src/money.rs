//! Money as integer minor units (SOP ENG-003).
//!
//! * Amounts are `i64` minor units (cents, sen). Never floats, never decimals
//!   at rest: storage is `BIGINT`, JSON is an integer.
//! * Every `Money` is within ±(2^53 − 1), so it survives JSON parsing in any
//!   language (JavaScript numbers are exact only up to 2^53 − 1).
//! * Rates are basis points and round half-to-even ("banker's rounding"), so
//!   rounding errors do not accumulate in one direction.

use std::cmp::Ordering;
use std::fmt;
use std::str::FromStr;

use serde::{Deserialize, Serialize};

/// Largest magnitude of a [`Money`] amount: 2^53 − 1.
pub const MAX_MINOR: i64 = 9_007_199_254_740_991;

/// Supported ISO 4217 currencies.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
pub enum Currency {
    #[serde(rename = "IDR")]
    Idr,
    #[serde(rename = "JPY")]
    Jpy,
    #[serde(rename = "SGD")]
    Sgd,
    #[serde(rename = "USD")]
    Usd,
}

impl Currency {
    pub const ALL: [Self; 4] = [Self::Idr, Self::Jpy, Self::Sgd, Self::Usd];

    /// ISO 4217 alphabetic code.
    pub const fn code(self) -> &'static str {
        match self {
            Self::Idr => "IDR",
            Self::Jpy => "JPY",
            Self::Sgd => "SGD",
            Self::Usd => "USD",
        }
    }

    /// ISO 4217 minor-unit exponent: amounts are stored as `major × 10^exp`.
    pub const fn minor_unit_exponent(self) -> u8 {
        match self {
            Self::Jpy => 0,
            Self::Idr | Self::Sgd | Self::Usd => 2,
        }
    }
}

impl fmt::Display for Currency {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.code())
    }
}

#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
#[error("unsupported currency `{0}`")]
pub struct UnknownCurrency(pub String);

impl FromStr for Currency {
    type Err = UnknownCurrency;

    fn from_str(code: &str) -> Result<Self, Self::Err> {
        Self::ALL
            .into_iter()
            .find(|currency| currency.code() == code)
            .ok_or_else(|| UnknownCurrency(code.to_owned()))
    }
}

#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum MoneyError {
    #[error("amount exceeds the supported range of ±{MAX_MINOR} minor units")]
    OutOfRange,
    #[error("cannot combine {left} with {right}")]
    CurrencyMismatch { left: Currency, right: Currency },
    #[error("rate must be between 0 and {max} basis points", max = Bps::MAX_VALUE)]
    InvalidRate,
}

/// An amount of money in one currency.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct Money {
    minor: i64,
    currency: Currency,
}

impl Money {
    pub fn from_minor(minor: i64, currency: Currency) -> Result<Self, MoneyError> {
        if minor.unsigned_abs() > MAX_MINOR.unsigned_abs() {
            return Err(MoneyError::OutOfRange);
        }
        Ok(Self { minor, currency })
    }

    pub const fn zero(currency: Currency) -> Self {
        Self { minor: 0, currency }
    }

    pub const fn minor(self) -> i64 {
        self.minor
    }

    pub const fn currency(self) -> Currency {
        self.currency
    }

    pub const fn is_negative(self) -> bool {
        self.minor < 0
    }

    pub fn checked_add(self, rhs: Self) -> Result<Self, MoneyError> {
        self.same_currency(rhs)?;
        let sum = self
            .minor
            .checked_add(rhs.minor)
            .ok_or(MoneyError::OutOfRange)?;
        Self::from_minor(sum, self.currency)
    }

    pub fn checked_sub(self, rhs: Self) -> Result<Self, MoneyError> {
        self.same_currency(rhs)?;
        let difference = self
            .minor
            .checked_sub(rhs.minor)
            .ok_or(MoneyError::OutOfRange)?;
        Self::from_minor(difference, self.currency)
    }

    pub fn checked_mul(self, factor: u32) -> Result<Self, MoneyError> {
        let product = self
            .minor
            .checked_mul(i64::from(factor))
            .ok_or(MoneyError::OutOfRange)?;
        Self::from_minor(product, self.currency)
    }

    /// `self × rate`, rounded half-to-even to a whole minor unit.
    pub fn apply_rate(self, rate: Bps) -> Result<Self, MoneyError> {
        let numerator = i128::from(self.minor)
            .checked_mul(i128::from(rate.get()))
            .ok_or(MoneyError::OutOfRange)?;
        let rounded = div_round_half_even(numerator, i128::from(Bps::MAX_VALUE))
            .ok_or(MoneyError::OutOfRange)?;
        let minor = i64::try_from(rounded).map_err(|_| MoneyError::OutOfRange)?;
        Self::from_minor(minor, self.currency)
    }

    fn same_currency(self, other: Self) -> Result<(), MoneyError> {
        if self.currency == other.currency {
            Ok(())
        } else {
            Err(MoneyError::CurrencyMismatch {
                left: self.currency,
                right: other.currency,
            })
        }
    }
}

/// A rate in basis points: `0..=10_000` (0 %..=100 %).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct Bps(u16);

impl Bps {
    pub const MAX_VALUE: u16 = 10_000;
    pub const ZERO: Self = Self(0);

    pub fn new(value: u16) -> Result<Self, MoneyError> {
        if value <= Self::MAX_VALUE {
            Ok(Self(value))
        } else {
            Err(MoneyError::InvalidRate)
        }
    }

    pub const fn get(self) -> u16 {
        self.0
    }
}

/// `numerator / denominator` rounded to the nearest integer, ties to even.
/// `None` only on overflow or a non-positive denominator.
fn div_round_half_even(numerator: i128, denominator: i128) -> Option<i128> {
    if denominator <= 0 {
        return None;
    }
    let quotient = numerator.checked_div(denominator)?; // truncates toward zero
    let remainder = numerator.checked_rem(denominator)?;
    let twice_remainder = remainder.checked_abs()?.checked_mul(2)?;
    let round_away_from_zero = match twice_remainder.cmp(&denominator) {
        Ordering::Less => false,
        Ordering::Greater => true,
        Ordering::Equal => quotient.checked_rem(2)? != 0,
    };
    if !round_away_from_zero {
        Some(quotient)
    } else if numerator.is_negative() {
        quotient.checked_sub(1)
    } else {
        quotient.checked_add(1)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn idr(minor: i64) -> Money {
        Money::from_minor(minor, Currency::Idr).unwrap()
    }

    #[test]
    fn half_even_rounding_table() {
        for (numerator, expected) in [
            (5, 0),  // 0.5 → 0
            (15, 2), // 1.5 → 2
            (25, 2), // 2.5 → 2
            (26, 3), // 2.6 → 3
            (-25, -2),
            (-35, -4),
            (24, 2),
        ] {
            assert_eq!(
                div_round_half_even(numerator, 10),
                Some(expected),
                "{numerator}/10"
            );
        }
    }

    #[test]
    fn rejects_amounts_outside_the_json_safe_range() {
        assert!(Money::from_minor(MAX_MINOR, Currency::Usd).is_ok());
        assert_eq!(
            Money::from_minor(MAX_MINOR + 1, Currency::Usd),
            Err(MoneyError::OutOfRange)
        );
        assert_eq!(
            Money::from_minor(i64::MIN, Currency::Usd),
            Err(MoneyError::OutOfRange)
        );
        assert_eq!(
            idr(MAX_MINOR).checked_add(idr(1)),
            Err(MoneyError::OutOfRange)
        );
    }

    #[test]
    fn refuses_to_mix_currencies() {
        let usd = Money::zero(Currency::Usd);

        assert!(matches!(
            idr(1).checked_add(usd),
            Err(MoneyError::CurrencyMismatch { .. })
        ));
    }

    #[test]
    fn eleven_percent_vat_example() {
        let subtotal = idr(30_000_000); // IDR 300,000.00

        let tax = subtotal.apply_rate(Bps::new(1_100).unwrap()).unwrap();

        assert_eq!(tax.minor(), 3_300_000);
    }

    #[test]
    fn rate_above_100_percent_is_invalid() {
        assert_eq!(Bps::new(10_001), Err(MoneyError::InvalidRate));
    }

    #[test]
    fn currency_codes_round_trip() {
        for currency in Currency::ALL {
            assert_eq!(currency.code().parse::<Currency>().unwrap(), currency);
        }
        assert!("XYZ".parse::<Currency>().is_err());
    }
}
