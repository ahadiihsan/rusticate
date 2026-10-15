//! Product-level tests (TEST-003): the real router, middleware and schema.
#![cfg(test)]

// rusticate:module(example):start
mod http_flows;
// rusticate:module(example):end
// rusticate:module(postgres):start
mod migrate;
// rusticate:module(postgres):end
mod openapi_contract;
// rusticate:module(postgres):start
mod schema_guard;
// rusticate:module(postgres):end
