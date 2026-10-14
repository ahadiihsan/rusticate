//! Billing rules as pure functions (SOP ENG-001..ENG-006).
//!
//! No async, no I/O, no clock, no randomness: same input, same output, always.
//! Everything here is tested exhaustively with unit tests and property tests,
//! in milliseconds, with no database.
//!
//! Integer overflow cannot be silent: arithmetic must go through checked
//! operations (`arithmetic_side_effects` is denied), and `as` casts are banned.
#![deny(clippy::arithmetic_side_effects, clippy::as_conversions)]

mod error;
pub mod invoice;
pub mod money;

pub use error::EngineError;
