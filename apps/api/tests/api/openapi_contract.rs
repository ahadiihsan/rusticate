//! API-004: the OpenAPI document is a committed, reviewed contract.
//! Regenerate with `cargo rusticate openapi`, then review the diff like code.

const COMMITTED: &str = include_str!("../../openapi.json");

#[test]
fn openapi_document_matches_the_committed_contract() {
    let generated = api::app::openapi_json().unwrap();

    assert!(
        generated == COMMITTED,
        "apps/api/openapi.json is stale: run `cargo rusticate openapi` and review the contract diff"
    );
}

#[test]
fn every_operation_has_an_explicit_id_and_security() {
    let document: serde_json::Value = serde_json::from_str(COMMITTED).unwrap();

    for (path, item) in document["paths"].as_object().unwrap() {
        for (method, operation) in item.as_object().unwrap() {
            assert!(
                operation["operationId"].is_string(),
                "{method} {path} lacks operationId"
            );
            assert!(
                operation["security"].is_array(),
                "{method} {path} lacks security"
            );
        }
    }
}
