//! End-to-end HTTP flows through the production router and middleware.

use std::sync::Arc;

use api::app::{self, AppState};
use axum::Router;
use axum::body::Body;
use axum::http::{HeaderMap, Method, Request, StatusCode, header};
use billing::permissions::USER_GRANTABLE;
use http_body_util::BodyExt as _;
use iam::{AuthConfig, TokenVerifier, dev};
use op_context::{ActorId, TenantId};
use secrecy::SecretString;
use serde_json::{Value, json};
use server_kit::{HttpConfig, Readiness};
use sqlx::postgres::{PgConnectOptions, PgPoolOptions};
use test_kit::TestDb;
use tower::ServiceExt as _;
use uuid::Uuid;

const SECRET: &str = "test-secret-test-secret-test-secret!";
const ISSUER: &str = "https://issuer.test";
const AUDIENCE: &str = "api";

struct Harness {
    router: Router,
}

struct Response {
    status: StatusCode,
    headers: HeaderMap,
    body: Value,
}

impl Harness {
    async fn new(pool: PgPoolOptions, conn: PgConnectOptions) -> Self {
        let db = TestDb::new(pool, conn, &[&billing::MIGRATOR]).await;
        let verifier = TokenVerifier::new(&AuthConfig {
            issuer: ISSUER.to_owned(),
            audience: AUDIENCE.to_owned(),
            public_key_pem: None,
            public_key_algorithm: iam::PublicKeyAlgorithm::RS256,
            hs256_secret: Some(SecretString::from(SECRET.to_owned())),
            leeway_secs: 0,
        })
        .unwrap();
        let router = app::build(
            AppState { db: db.app.clone() },
            Arc::new(verifier),
            &Readiness::new(Vec::new()),
            &HttpConfig::default(),
        );
        Self { router }
    }

    async fn send(
        &self,
        method: Method,
        uri: &str,
        token: Option<&str>,
        headers: &[(&str, &str)],
        body: Option<Value>,
    ) -> Response {
        let mut request = Request::builder().method(method).uri(uri);
        if let Some(token) = token {
            request = request.header(header::AUTHORIZATION, format!("Bearer {token}"));
        }
        for (name, value) in headers {
            request = request.header(*name, *value);
        }
        let request = match body {
            Some(json) => request
                .header(header::CONTENT_TYPE, "application/json")
                .body(Body::from(json.to_string())),
            None => request.body(Body::empty()),
        }
        .unwrap();
        let response = self.router.clone().oneshot(request).await.unwrap();
        let status = response.status();
        let headers = response.headers().clone();
        let bytes = response.into_body().collect().await.unwrap().to_bytes();
        Response {
            status,
            headers,
            body: serde_json::from_slice(&bytes).unwrap_or(Value::Null),
        }
    }
}

fn token(tenant: TenantId, permissions: &[&str]) -> String {
    dev::issue_hs256(&dev::DevToken {
        secret: SECRET,
        issuer: ISSUER,
        audience: AUDIENCE,
        tenant,
        actor: ActorId::from_uuid(Uuid::now_v7()),
        permissions,
        ttl_secs: 300,
    })
    .unwrap()
}

fn admin_token(tenant: TenantId) -> String {
    let all: Vec<&str> = USER_GRANTABLE.iter().map(|p| p.as_str()).collect();
    token(tenant, &all)
}

fn invoice_body() -> Value {
    json!({
        "customer_email": "ap@customer.example",
        "lines": [{ "description": "Consulting", "quantity": 2, "unit_price_minor": 15_000_000 }]
    })
}

async fn configured_tenant(harness: &Harness) -> (TenantId, String) {
    let tenant = TenantId::from_uuid(Uuid::now_v7());
    let token = admin_token(tenant);
    let created = harness
        .send(
            Method::PUT,
            "/v1/billing/settings",
            Some(&token),
            &[("if-none-match", "*")],
            Some(json!({ "currency": "IDR", "tax_rate_bps": 1100 })),
        )
        .await;
    assert_eq!(created.status, StatusCode::CREATED, "{}", created.body);
    assert_eq!(created.headers[header::ETAG], "\"1\"");
    (tenant, token)
}

#[sqlx::test(migrations = false)]
async fn unauthenticated_requests_get_problem_json_401(
    pool: PgPoolOptions,
    conn: PgConnectOptions,
) {
    let harness = Harness::new(pool, conn).await;

    let response = harness
        .send(Method::GET, "/v1/invoices", None, &[], None)
        .await;

    assert_eq!(response.status, StatusCode::UNAUTHORIZED);
    assert_eq!(
        response.headers[header::CONTENT_TYPE],
        "application/problem+json"
    );
    assert_eq!(response.body["error_code"], "UNAUTHENTICATED");
    assert_eq!(response.body["instance"], "/v1/invoices");
    assert!(response.headers.contains_key("x-request-id"));
}

#[sqlx::test(migrations = false)]
async fn issue_is_idempotent_and_reports_location(pool: PgPoolOptions, conn: PgConnectOptions) {
    let harness = Harness::new(pool, conn).await;
    let (_, token) = configured_tenant(&harness).await;
    let key = [("idempotency-key", "6c1f5f0e-order-42")];

    let first = harness
        .send(
            Method::POST,
            "/v1/invoices",
            Some(&token),
            &key,
            Some(invoice_body()),
        )
        .await;
    let retry = harness
        .send(
            Method::POST,
            "/v1/invoices",
            Some(&token),
            &key,
            Some(invoice_body()),
        )
        .await;

    assert_eq!(first.status, StatusCode::CREATED, "{}", first.body);
    assert_eq!(first.body["total_minor"], 33_300_000);
    assert_eq!(
        first.headers[header::LOCATION],
        format!("/v1/invoices/{}", first.body["id"].as_str().unwrap())
    );
    assert_eq!(retry.status, StatusCode::CREATED);
    assert_eq!(retry.body["id"], first.body["id"]);
}

#[sqlx::test(migrations = false)]
async fn mutations_without_their_required_headers_are_rejected(
    pool: PgPoolOptions,
    conn: PgConnectOptions,
) {
    let harness = Harness::new(pool, conn).await;
    let (_, token) = configured_tenant(&harness).await;

    let no_key = harness
        .send(
            Method::POST,
            "/v1/invoices",
            Some(&token),
            &[],
            Some(invoice_body()),
        )
        .await;
    let no_precondition = harness
        .send(
            Method::PUT,
            "/v1/billing/settings",
            Some(&token),
            &[],
            Some(json!({ "currency": "USD", "tax_rate_bps": 0 })),
        )
        .await;

    assert_eq!(no_key.status, StatusCode::BAD_REQUEST);
    assert_eq!(no_key.body["error_code"], "IDEMPOTENCY_KEY_REQUIRED");
    assert_eq!(no_precondition.status, StatusCode::PRECONDITION_REQUIRED);
}

#[sqlx::test(migrations = false)]
async fn validation_errors_list_every_field(pool: PgPoolOptions, conn: PgConnectOptions) {
    let harness = Harness::new(pool, conn).await;
    let (_, token) = configured_tenant(&harness).await;
    let body = json!({ "customer_email": "nope", "lines": [{ "description": "", "quantity": 0, "unit_price_minor": -1 }] });

    let response = harness
        .send(
            Method::POST,
            "/v1/invoices",
            Some(&token),
            &[("idempotency-key", "k")],
            Some(body),
        )
        .await;

    assert_eq!(response.status, StatusCode::BAD_REQUEST);
    assert_eq!(response.body["error_code"], "VALIDATION_FAILED");
    assert_eq!(response.body["errors"].as_array().unwrap().len(), 4);
}

#[sqlx::test(migrations = false)]
async fn void_requires_the_current_etag(pool: PgPoolOptions, conn: PgConnectOptions) {
    let harness = Harness::new(pool, conn).await;
    let (_, token) = configured_tenant(&harness).await;
    let issued = harness
        .send(
            Method::POST,
            "/v1/invoices",
            Some(&token),
            &[("idempotency-key", "k1")],
            Some(invoice_body()),
        )
        .await;
    let void_uri = format!("/v1/invoices/{}/void", issued.body["id"].as_str().unwrap());
    let reason = json!({ "reason": "duplicate" });

    let stale = harness
        .send(
            Method::POST,
            &void_uri,
            Some(&token),
            &[("if-match", "\"9\"")],
            Some(reason.clone()),
        )
        .await;
    let current = harness
        .send(
            Method::POST,
            &void_uri,
            Some(&token),
            &[("if-match", "\"1\"")],
            Some(reason),
        )
        .await;

    assert_eq!(stale.status, StatusCode::PRECONDITION_FAILED);
    assert_eq!(current.status, StatusCode::OK, "{}", current.body);
    assert_eq!(current.body["status"], "void");
    assert_eq!(current.headers[header::ETAG], "\"2\"");
}

#[sqlx::test(migrations = false)]
async fn tenants_cannot_reach_each_others_invoices(pool: PgPoolOptions, conn: PgConnectOptions) {
    let harness = Harness::new(pool, conn).await;
    let (_, owner) = configured_tenant(&harness).await;
    let issued = harness
        .send(
            Method::POST,
            "/v1/invoices",
            Some(&owner),
            &[("idempotency-key", "k1")],
            Some(invoice_body()),
        )
        .await;
    let uri = format!("/v1/invoices/{}", issued.body["id"].as_str().unwrap());

    let intruder = admin_token(TenantId::from_uuid(Uuid::now_v7()));
    let response = harness
        .send(Method::GET, &uri, Some(&intruder), &[], None)
        .await;

    assert_eq!(response.status, StatusCode::NOT_FOUND);
}

#[sqlx::test(migrations = false)]
async fn missing_permission_is_403(pool: PgPoolOptions, conn: PgConnectOptions) {
    let harness = Harness::new(pool, conn).await;
    let (tenant, _) = configured_tenant(&harness).await;
    let reader = token(tenant, &["billing.invoice.read"]);

    let response = harness
        .send(
            Method::POST,
            "/v1/invoices",
            Some(&reader),
            &[("idempotency-key", "k")],
            Some(invoice_body()),
        )
        .await;

    assert_eq!(response.status, StatusCode::FORBIDDEN);
    assert_eq!(response.body["error_code"], "FORBIDDEN");
}

#[sqlx::test(migrations = false)]
async fn settings_and_stats_are_readable(pool: PgPoolOptions, conn: PgConnectOptions) {
    let harness = Harness::new(pool, conn).await;
    let unconfigured = admin_token(TenantId::from_uuid(Uuid::now_v7()));
    let (_, token) = configured_tenant(&harness).await;

    let missing = harness
        .send(
            Method::GET,
            "/v1/billing/settings",
            Some(&unconfigured),
            &[],
            None,
        )
        .await;
    let settings = harness
        .send(Method::GET, "/v1/billing/settings", Some(&token), &[], None)
        .await;
    let stats = harness
        .send(Method::GET, "/v1/billing/stats", Some(&token), &[], None)
        .await;

    assert_eq!(missing.status, StatusCode::NOT_FOUND);
    assert_eq!(settings.status, StatusCode::OK);
    assert_eq!(settings.headers[header::ETAG], "\"1\"");
    assert_eq!(settings.body["currency"], "IDR");
    assert_eq!(stats.status, StatusCode::OK);
    assert_eq!(
        stats.body["currencies"],
        json!([]),
        "eventually consistent: no consumer ran"
    );
}

#[sqlx::test(migrations = false)]
async fn invoices_are_listed_in_pages_and_fetched_with_their_etag(
    pool: PgPoolOptions,
    conn: PgConnectOptions,
) {
    let harness = Harness::new(pool, conn).await;
    let (_, token) = configured_tenant(&harness).await;
    for key in ["k1", "k2", "k3"] {
        harness
            .send(
                Method::POST,
                "/v1/invoices",
                Some(&token),
                &[("idempotency-key", key)],
                Some(invoice_body()),
            )
            .await;
    }

    let first = harness
        .send(Method::GET, "/v1/invoices?limit=2", Some(&token), &[], None)
        .await;
    let cursor = first.body["next_cursor"].as_str().unwrap().to_owned();
    let second = harness
        .send(
            Method::GET,
            &format!("/v1/invoices?limit=2&cursor={cursor}"),
            Some(&token),
            &[],
            None,
        )
        .await;
    let bad_filter = harness
        .send(
            Method::GET,
            "/v1/invoices?status=lost",
            Some(&token),
            &[],
            None,
        )
        .await;
    let id = first.body["items"][0]["id"].as_str().unwrap().to_owned();
    let one = harness
        .send(
            Method::GET,
            &format!("/v1/invoices/{id}"),
            Some(&token),
            &[],
            None,
        )
        .await;

    assert_eq!(first.body["items"].as_array().unwrap().len(), 2);
    assert_eq!(second.body["items"].as_array().unwrap().len(), 1);
    assert!(
        second.body.get("next_cursor").is_none(),
        "last page has no cursor"
    );
    assert_eq!(bad_filter.status, StatusCode::BAD_REQUEST);
    assert_eq!(one.status, StatusCode::OK);
    assert_eq!(one.headers[header::ETAG], "\"1\"");
    assert_eq!(one.body["lines"][0]["line_total_minor"], 30_000_000);
}
