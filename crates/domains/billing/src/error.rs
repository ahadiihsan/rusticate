use api_kit::{ApiError, precondition_failed};
use billing_engine::EngineError;
use billing_engine::money::MoneyError;
use datastore::DataStoreError;
use idempotency::IdempotencyError;
use messaging::HandlerError;
use op_context::AuthzError;

/// Everything a billing workflow can fail with (ERR-001).
///
/// Workflows return this, never `ApiError`: they are also called by event
/// consumers and jobs. The HTTP mapping lives in one place, below.
#[derive(Debug, thiserror::Error)]
pub enum BillingError {
    #[error(transparent)]
    Forbidden(#[from] AuthzError),
    #[error("billing is not configured for this tenant")]
    NotConfigured,
    #[error("billing settings already exist")]
    SettingsAlreadyExist,
    #[error("invoice not found")]
    InvoiceNotFound,
    #[error("the resource was modified by another request")]
    StaleVersion,
    #[error(transparent)]
    Rule(#[from] EngineError),
    #[error(transparent)]
    Idempotency(#[from] IdempotencyError),
    #[error(transparent)]
    Store(#[from] DataStoreError),
    #[error("stored data is inconsistent: {0}")]
    Corrupt(String),
}

impl From<sqlx::Error> for BillingError {
    fn from(err: sqlx::Error) -> Self {
        Self::Store(err.into())
    }
}

impl From<MoneyError> for BillingError {
    fn from(err: MoneyError) -> Self {
        Self::Rule(err.into())
    }
}

impl From<serde_json::Error> for BillingError {
    fn from(err: serde_json::Error) -> Self {
        Self::Store(err.into())
    }
}

impl From<BillingError> for ApiError {
    fn from(err: BillingError) -> Self {
        match err {
            BillingError::Forbidden(authz) => authz.into(),
            BillingError::NotConfigured => Self::unprocessable(
                "BILLING_NOT_CONFIGURED",
                "Configure billing settings (PUT /v1/billing/settings) before issuing invoices.",
            ),
            BillingError::SettingsAlreadyExist => {
                precondition_failed("Billing settings already exist; update them with If-Match.")
            }
            BillingError::InvoiceNotFound => Self::not_found("invoice"),
            BillingError::StaleVersion => precondition_failed(
                "The resource changed since it was read. Fetch it again and retry with the new ETag.",
            ),
            BillingError::Rule(rule) => rule_violation(&rule),
            BillingError::Idempotency(idempotency) => idempotency.into(),
            BillingError::Store(store) => store.into(),
            corrupt @ BillingError::Corrupt(_) => Self::internal(corrupt),
        }
    }
}

fn rule_violation(rule: &EngineError) -> ApiError {
    let code = match rule {
        EngineError::InvalidTransition { .. } => {
            return ApiError::conflict("INVALID_STATE_TRANSITION", rule.to_string());
        }
        EngineError::NoLines => "INVOICE_HAS_NO_LINES",
        EngineError::TooManyLines { .. } => "INVOICE_TOO_MANY_LINES",
        EngineError::CurrencyMismatch { .. }
        | EngineError::Money(MoneyError::CurrencyMismatch { .. }) => "CURRENCY_MISMATCH",
        EngineError::NegativePrice { .. } => "NEGATIVE_PRICE",
        EngineError::Money(MoneyError::OutOfRange) => "AMOUNT_OUT_OF_RANGE",
        EngineError::Money(MoneyError::InvalidRate) => "INVALID_RATE",
    };
    ApiError::unprocessable(code, rule.to_string())
}

/// For event consumers: retry only what can succeed on retry.
impl From<BillingError> for HandlerError {
    fn from(err: BillingError) -> Self {
        match err {
            BillingError::Store(store) => store.into(),
            other => Self::permanent(other),
        }
    }
}

#[cfg(test)]
mod tests {
    use billing_engine::invoice::{InvoiceAction, InvoiceStatus};
    use billing_engine::money::Currency;
    use op_context::Permission;

    use super::*;

    fn http(err: BillingError) -> (u16, &'static str) {
        let api = ApiError::from(err);
        (api.status().as_u16(), api.code())
    }

    #[test]
    fn every_error_has_a_deliberate_http_mapping() {
        let rule = |e: EngineError| BillingError::Rule(e);
        let cases = [
            (
                BillingError::Forbidden(AuthzError::MissingPermission {
                    required: Permission::new("billing.invoice.issue"),
                }),
                403,
                "FORBIDDEN",
            ),
            (BillingError::NotConfigured, 422, "BILLING_NOT_CONFIGURED"),
            (
                BillingError::SettingsAlreadyExist,
                412,
                "PRECONDITION_FAILED",
            ),
            (BillingError::InvoiceNotFound, 404, "RESOURCE_NOT_FOUND"),
            (BillingError::StaleVersion, 412, "PRECONDITION_FAILED"),
            (
                rule(EngineError::InvalidTransition {
                    from: InvoiceStatus::Void,
                    action: InvoiceAction::Void,
                }),
                409,
                "INVALID_STATE_TRANSITION",
            ),
            (rule(EngineError::NoLines), 422, "INVOICE_HAS_NO_LINES"),
            (
                rule(EngineError::TooManyLines { max: 1 }),
                422,
                "INVOICE_TOO_MANY_LINES",
            ),
            (
                rule(EngineError::CurrencyMismatch {
                    line: 0,
                    expected: Currency::Idr,
                    found: Currency::Usd,
                }),
                422,
                "CURRENCY_MISMATCH",
            ),
            (
                rule(EngineError::NegativePrice { line: 0 }),
                422,
                "NEGATIVE_PRICE",
            ),
            (
                rule(EngineError::Money(MoneyError::OutOfRange)),
                422,
                "AMOUNT_OUT_OF_RANGE",
            ),
            (
                rule(EngineError::Money(MoneyError::InvalidRate)),
                422,
                "INVALID_RATE",
            ),
            (
                BillingError::Idempotency(IdempotencyError::KeyReused),
                422,
                "IDEMPOTENCY_KEY_REUSED",
            ),
            (
                BillingError::Store(DataStoreError::LockTimeout),
                503,
                "CONCURRENT_MODIFICATION",
            ),
            (
                BillingError::Corrupt("bad row".to_owned()),
                500,
                "INTERNAL_ERROR",
            ),
        ];
        for (err, status, code) in cases {
            let label = err.to_string();
            assert_eq!(http(err), (status, code), "{label}");
        }
    }

    #[test]
    fn consumers_retry_only_transient_failures() {
        assert!(
            HandlerError::from(BillingError::Store(DataStoreError::LockTimeout)).is_transient()
        );
        assert!(!HandlerError::from(BillingError::NotConfigured).is_transient());
        assert!(!HandlerError::from(BillingError::Corrupt("x".to_owned())).is_transient());
    }
}
