//! Typed worker configuration from `APP_*` environment variables.

use std::num::NonZeroUsize;

use datastore::DataStoreConfig;
use figment::Figment;
use figment::providers::Env;
use serde::Deserialize;
use server_kit::HttpConfig;
use telemetry::TelemetryConfig;

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Config {
    /// Health probes only (`/health/live`, `/health/ready`). Defaults to :8081.
    #[serde(default = "default_http")]
    pub http: HttpConfig,
    /// RLS-bound `app_rw` connection for event handlers.
    pub database: DataStoreConfig,
    /// Cross-tenant `app_system` connection for the relay and maintenance.
    pub system_database: DataStoreConfig,
    pub nats: NatsConfig,
    #[serde(default)]
    pub consumers: ConsumerConfig,
    #[serde(default)]
    pub telemetry: TelemetryConfig,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct NatsConfig {
    pub url: String,
    /// JetStream replicas for the events stream (3 in production clusters).
    #[serde(default = "default_replicas")]
    pub stream_replicas: usize,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ConsumerConfig {
    /// Messages processed concurrently per consumer and replica.
    #[serde(default = "default_concurrency")]
    pub concurrency: NonZeroUsize,
    /// Seconds between platform maintenance runs.
    #[serde(default = "default_maintenance_interval_secs")]
    pub maintenance_interval_secs: u64,
}

impl Default for ConsumerConfig {
    fn default() -> Self {
        Self {
            concurrency: default_concurrency(),
            maintenance_interval_secs: default_maintenance_interval_secs(),
        }
    }
}

fn default_http() -> HttpConfig {
    HttpConfig {
        bind: std::net::SocketAddr::from(([0, 0, 0, 0], 8081)),
        ..HttpConfig::default()
    }
}

const fn default_replicas() -> usize {
    1
}

const fn default_concurrency() -> NonZeroUsize {
    NonZeroUsize::MIN.saturating_add(15)
}

const fn default_maintenance_interval_secs() -> u64 {
    60
}

impl Config {
    /// Loads from `APP_*` environment variables (`__` separates nesting).
    pub fn load() -> anyhow::Result<Self> {
        Self::from_figment(&Figment::new().merge(Env::prefixed("APP_").split("__")))
    }

    pub fn from_figment(figment: &Figment) -> anyhow::Result<Self> {
        Ok(figment.extract()?)
    }
}

#[cfg(test)]
mod tests {
    use figment::providers::{Format as _, Toml};

    use super::*;

    const BASE: &str = r#"
        [database]
        url = "postgres://app_rw@localhost/db"
        [system_database]
        url = "postgres://app_system@localhost/db"
        [nats]
        url = "nats://localhost:4222"
    "#;

    #[test]
    fn defaults_put_probes_on_8081_and_bound_concurrency() {
        let config = Config::from_figment(&Figment::from(Toml::string(BASE))).unwrap();

        assert_eq!(config.http.bind.port(), 8081);
        assert_eq!(config.consumers.concurrency.get(), 16);
        assert_eq!(config.nats.stream_replicas, 1);
    }

    #[test]
    fn missing_or_unknown_keys_are_rejected() {
        let typo = Figment::from(Toml::string(BASE)).merge(Toml::string("[nats]\nurls = \"x\""));

        assert!(Config::from_figment(&Figment::from(Toml::string("[nats]\nurl = \"x\""))).is_err());
        assert!(Config::from_figment(&typo).is_err());
        let unbounded =
            Figment::from(Toml::string(BASE)).merge(Toml::string("[consumers]\nconcurrency = 0"));
        assert!(
            Config::from_figment(&unbounded).is_err(),
            "0 would mean unbounded concurrency"
        );
    }
}
