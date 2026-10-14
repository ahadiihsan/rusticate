use crate::invoice::{InvoiceAction, InvoiceStatus};
use crate::money::{Currency, MoneyError};

/// Every way a billing rule can reject its input.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum EngineError {
    #[error("an invoice needs at least one line")]
    NoLines,
    #[error("an invoice may have at most {max} lines")]
    TooManyLines { max: usize },
    #[error("line {line} is priced in {found}, but the invoice currency is {expected}")]
    CurrencyMismatch {
        line: usize,
        expected: Currency,
        found: Currency,
    },
    #[error("line {line} has a negative unit price")]
    NegativePrice { line: usize },
    #[error("cannot {action} an invoice that is {from}")]
    InvalidTransition {
        from: InvoiceStatus,
        action: InvoiceAction,
    },
    #[error(transparent)]
    Money(#[from] MoneyError),
}
