//! Worker end-to-end tests against real Postgres and a real NATS JetStream
//! (`cargo rusticate db up` starts both; CI runs them as services).
#![cfg(test)]

mod health;
// rusticate:module(example):start
mod pipeline;
mod wiring;
// rusticate:module(example):end
