use std::time::Duration;

use secrecy::SecretString;
use serde::Deserialize;

/// Connection pool settings for one database role.
#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct DataStoreConfig {
    /// `postgres://` URL. Secret: never logged.
    pub url: SecretString,
    #[serde(default = "defaults::max_connections")]
    pub max_connections: u32,
    #[serde(default = "defaults::min_connections")]
    pub min_connections: u32,
    /// How long a request may wait for a free connection before failing with 503.
    #[serde(default = "defaults::acquire_timeout_ms")]
    pub acquire_timeout_ms: u64,
    /// Server-side cap on any single statement.
    #[serde(default = "defaults::statement_timeout_ms")]
    pub statement_timeout_ms: u64,
    /// Server-side cap on waiting for a row lock (e.g. an in-flight idempotent duplicate).
    #[serde(default = "defaults::lock_timeout_ms")]
    pub lock_timeout_ms: u64,
    /// Kills sessions that leave a transaction open and idle (a leaked `TenantTx`).
    #[serde(default = "defaults::idle_in_transaction_timeout_ms")]
    pub idle_in_transaction_timeout_ms: u64,
}

impl DataStoreConfig {
    pub fn new(url: SecretString) -> Self {
        Self {
            url,
            max_connections: defaults::max_connections(),
            min_connections: defaults::min_connections(),
            acquire_timeout_ms: defaults::acquire_timeout_ms(),
            statement_timeout_ms: defaults::statement_timeout_ms(),
            lock_timeout_ms: defaults::lock_timeout_ms(),
            idle_in_transaction_timeout_ms: defaults::idle_in_transaction_timeout_ms(),
        }
    }

    pub(crate) fn acquire_timeout(&self) -> Duration {
        Duration::from_millis(self.acquire_timeout_ms)
    }

    /// Session parameters sent in the startup packet (no extra round trip).
    pub(crate) fn session_options(&self) -> [(&'static str, String); 3] {
        [
            (
                "statement_timeout",
                format!("{}ms", self.statement_timeout_ms),
            ),
            ("lock_timeout", format!("{}ms", self.lock_timeout_ms)),
            (
                "idle_in_transaction_session_timeout",
                format!("{}ms", self.idle_in_transaction_timeout_ms),
            ),
        ]
    }
}

mod defaults {
    pub(super) const fn max_connections() -> u32 {
        20
    }
    pub(super) const fn min_connections() -> u32 {
        2
    }
    pub(super) const fn acquire_timeout_ms() -> u64 {
        3_000
    }
    pub(super) const fn statement_timeout_ms() -> u64 {
        5_000
    }
    pub(super) const fn lock_timeout_ms() -> u64 {
        3_000
    }
    pub(super) const fn idle_in_transaction_timeout_ms() -> u64 {
        15_000
    }
}

#[cfg(test)]
mod tests {
    use secrecy::SecretString;

    use super::*;

    #[test]
    fn defaults_bound_every_wait() {
        let cfg = DataStoreConfig::new(SecretString::from("postgres://x".to_owned()));

        assert_eq!(cfg.acquire_timeout(), Duration::from_secs(3));
        assert_eq!(
            cfg.session_options()
                .map(|(key, value)| format!("{key}={value}")),
            [
                "statement_timeout=5000ms",
                "lock_timeout=3000ms",
                "idle_in_transaction_session_timeout=15000ms",
            ]
        );
    }

    #[test]
    fn deserializes_with_defaults_and_rejects_unknown_keys() {
        let cfg: DataStoreConfig =
            serde_json::from_str(r#"{"url":"postgres://x","max_connections":7}"#).unwrap();
        let typo =
            serde_json::from_str::<DataStoreConfig>(r#"{"url":"postgres://x","max_conections":7}"#);

        assert_eq!(cfg.max_connections, 7);
        assert_eq!(cfg.min_connections, 2);
        assert!(typo.is_err());
    }
}
