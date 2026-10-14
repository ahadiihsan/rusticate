//! Readiness check for the broker connection.

use async_nats::connection::State;
use futures::future::BoxFuture;
use server_kit::ReadinessCheck;

#[derive(Debug, Clone)]
pub struct NatsReadiness(pub async_nats::Client);

impl ReadinessCheck for NatsReadiness {
    fn name(&self) -> &'static str {
        "nats"
    }

    fn check(&self) -> BoxFuture<'_, Result<(), String>> {
        let state = self.0.connection_state();
        Box::pin(async move {
            match state {
                State::Connected => Ok(()),
                other => Err(format!("connection state: {other:?}")),
            }
        })
    }
}
