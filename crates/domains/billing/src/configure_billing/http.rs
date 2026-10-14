//! HOP 1: HTTP adapter for `configure_billing`.

use api_kit::{
    ApiError, ApiResult, FieldViolation, Json, Precondition, ProblemDetails, precondition_required,
};
use axum::extract::State;
use axum::http::StatusCode;
use axum::response::IntoResponse;
use billing_engine::money::{Bps, Currency};
use datastore::DataStore;
use op_context::OpCtx;
use serde::Deserialize;
use utoipa::ToSchema;

use super::{ConfigureBillingCmd, Expectation, configure_billing};
use crate::dto::{BillingSettingsResponse, etag_header};

/// Billing settings to create or replace.
#[derive(Debug, Deserialize, ToSchema)]
#[serde(deny_unknown_fields)]
pub(crate) struct ConfigureBillingRequest {
    /// ISO 4217 code: IDR, JPY, SGD or USD.
    #[schema(example = "IDR")]
    currency: String,
    /// Tax rate in basis points, 0..=10000 (1100 = 11 %).
    #[schema(example = 1100)]
    tax_rate_bps: u32,
}

impl ConfigureBillingRequest {
    /// Parse, don't validate: every violation is reported at once.
    fn parse(self, precondition: Precondition) -> ApiResult<ConfigureBillingCmd> {
        let expectation = match precondition {
            Precondition::IfNoneMatchAny => Expectation::Create,
            Precondition::IfMatch(version) => Expectation::Update { version },
            Precondition::None => {
                return Err(precondition_required(
                    "Send `If-None-Match: *` to create settings or `If-Match: <ETag>` to update them.",
                ));
            }
        };

        let mut violations = Vec::new();
        let currency = match self.currency.parse::<Currency>() {
            Ok(currency) => Some(currency),
            Err(err) => {
                violations.push(FieldViolation::new("currency", err.to_string()));
                None
            }
        };
        let tax_rate = u16::try_from(self.tax_rate_bps)
            .ok()
            .and_then(|bps| Bps::new(bps).ok());
        if tax_rate.is_none() {
            violations.push(FieldViolation::new(
                "tax_rate_bps",
                "must be between 0 and 10000",
            ));
        }

        match (currency, tax_rate) {
            (Some(currency), Some(tax_rate)) => Ok(ConfigureBillingCmd {
                currency,
                tax_rate,
                expectation,
            }),
            _ => Err(ApiError::validation_many(violations)),
        }
    }
}

/// Create (`If-None-Match: *`) or update (`If-Match`) the tenant's billing settings.
#[utoipa::path(
    put,
    path = "/v1/billing/settings",
    operation_id = "configureBilling",
    tag = "billing",
    request_body = ConfigureBillingRequest,
    params(
        ("If-Match" = Option<String>, Header, description = "Current ETag, to update existing settings."),
        ("If-None-Match" = Option<String>, Header, description = "`*`, to create settings."),
    ),
    responses(
        (status = 200, description = "Settings updated.", body = BillingSettingsResponse, headers(("ETag" = String))),
        (status = 201, description = "Settings created.", body = BillingSettingsResponse, headers(("ETag" = String))),
        (status = 400, description = "Invalid request.", body = ProblemDetails, content_type = "application/problem+json"),
        (status = 403, description = "Missing `billing.settings.manage`.", body = ProblemDetails, content_type = "application/problem+json"),
        (status = 412, description = "Stale ETag, or settings already exist.", body = ProblemDetails, content_type = "application/problem+json"),
        (status = 428, description = "No conditional header sent.", body = ProblemDetails, content_type = "application/problem+json"),
    ),
    security(("bearer" = [])),
)]
pub(crate) async fn handle(
    ctx: OpCtx,
    State(db): State<DataStore>,
    precondition: Precondition,
    Json(body): Json<ConfigureBillingRequest>,
) -> ApiResult<impl IntoResponse> {
    let cmd = body.parse(precondition)?;
    let status = match cmd.expectation {
        Expectation::Create => StatusCode::CREATED,
        Expectation::Update { .. } => StatusCode::OK,
    };
    let settings = configure_billing(&ctx, &db, cmd).await?;
    Ok((
        status,
        etag_header(settings.version),
        Json(BillingSettingsResponse::from(settings)),
    ))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn request(currency: &str, tax_rate_bps: u32) -> ConfigureBillingRequest {
        ConfigureBillingRequest {
            currency: currency.to_owned(),
            tax_rate_bps,
        }
    }

    #[test]
    fn maps_preconditions_to_expectations() {
        let create = request("IDR", 1100)
            .parse(Precondition::IfNoneMatchAny)
            .unwrap();
        let update = request("IDR", 1100)
            .parse(Precondition::IfMatch(4))
            .unwrap();

        assert_eq!(create.expectation, Expectation::Create);
        assert_eq!(update.expectation, Expectation::Update { version: 4 });
    }

    #[test]
    fn requires_a_precondition() {
        let err = request("IDR", 1100).parse(Precondition::None).unwrap_err();

        assert_eq!(err.status(), StatusCode::PRECONDITION_REQUIRED);
    }

    #[test]
    fn reports_every_invalid_field() {
        let err = request("XXX", 20_000)
            .parse(Precondition::IfNoneMatchAny)
            .unwrap_err();

        let ApiError::BadRequest { violations, .. } = err else {
            panic!("expected a validation error");
        };
        let fields: Vec<_> = violations.iter().map(|v| v.field.as_str()).collect();
        assert_eq!(fields, ["currency", "tax_rate_bps"]);
    }
}
