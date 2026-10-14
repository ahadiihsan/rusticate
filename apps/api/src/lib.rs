//! Product API composition root (ARCH-008): configuration, wiring, routing.
//! No business logic and no SQL live here. Exposed as a library so the
//! integration tests exercise exactly the router production serves.

pub mod app;
pub mod config;
pub mod migrate;
pub mod openapi;
