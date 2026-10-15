//! Root OpenAPI document. Operations are collected from the mounted
//! contexts' routers; this adds metadata, security and shared schemas.

use api_kit::{FieldViolation, ProblemDetails};
use utoipa::openapi::security::{HttpAuthScheme, HttpBuilder, SecurityScheme};
use utoipa::{Modify, OpenApi};

#[derive(Debug, OpenApi)]
#[openapi(
    info(
        title = "Rusticate Product API",
        description = "Multi-tenant product API. Every error is RFC 9457 `application/problem+json`.",
        license(name = "Proprietary"),
    ),
    modifiers(&BearerAuth),
    components(schemas(ProblemDetails, FieldViolation)),
    // rusticate:module(example):start
    tags((name = "billing", description = "Tenant billing settings and invoices.")),
    // rusticate:module(example):end
)]
pub struct ApiDoc;

struct BearerAuth;

impl Modify for BearerAuth {
    fn modify(&self, openapi: &mut utoipa::openapi::OpenApi) {
        let components = openapi.components.get_or_insert_with(Default::default);
        components.add_security_scheme(
            "bearer",
            SecurityScheme::Http(
                HttpBuilder::new()
                    .scheme(HttpAuthScheme::Bearer)
                    .bearer_format("JWT")
                    .build(),
            ),
        );
    }
}
