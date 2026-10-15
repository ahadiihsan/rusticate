//! Typed configuration (CFG-001..CFG-003): read once at startup from
//! `APP_*` environment variables (`__` separates nesting levels, e.g.
//! `APP_DATABASE__URL`), validated, then passed down explicitly. Nothing else
//! in the process reads the environment.

// rusticate:module(postgres):start
use datastore::DataStoreConfig;
// rusticate:module(postgres):end
use figment::Figment;
use figment::providers::Env;
use iam::AuthConfig;
// rusticate:module(postgres):start
use secrecy::SecretString;
// rusticate:module(postgres):end
use serde::Deserialize;
use server_kit::HttpConfig;
use telemetry::TelemetryConfig;

/// Deployment environment. Stricter rules apply outside `Local`.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Environment {
    #[default]
    Local,
    Staging,
    Production,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Config {
    #[serde(default)]
    pub environment: Environment,
    #[serde(default)]
    pub http: HttpConfig,
    // rusticate:module(postgres):start
    /// Runtime connection: the RLS-bound `app_rw` role.
    pub database: DataStoreConfig,
    /// Owner connection, used only by the `migrate` command.
    #[serde(default)]
    pub migration_database_url: Option<SecretString>,
    // rusticate:module(postgres):end
    pub auth: AuthConfig,
    #[serde(default)]
    pub telemetry: TelemetryConfig,
}

impl Config {
    /// Loads and validates. Fails fast with every problem in the message.
    pub fn load() -> anyhow::Result<Self> {
        Self::from_figment(&Figment::new().merge(Env::prefixed("APP_").split("__")))
    }

    pub fn from_figment(figment: &Figment) -> anyhow::Result<Self> {
        let config: Self = figment.extract()?;
        config.validate()?;
        Ok(config)
    }

    fn validate(&self) -> anyhow::Result<()> {
        let mut problems = Vec::new();
        if self.environment != Environment::Local {
            if self.auth.hs256_secret.is_some() {
                problems.push(
                    "auth.hs256_secret is for local development only; use auth.public_key_pem",
                );
            }
            if self.http.docs_ui {
                problems.push("http.docs_ui must be off outside local development");
            }
        }
        if self
            .http
            .drain_delay_ms
            .saturating_add(self.http.shutdown_timeout_ms)
            > 55_000
        {
            problems.push("http.drain_delay_ms + http.shutdown_timeout_ms must stay below the pod grace period (60s)");
        }
        anyhow::ensure!(
            problems.is_empty(),
            "invalid configuration: {}",
            problems.join("; ")
        );
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use figment::providers::{Format as _, Toml};

    use super::*;

    const BASE: &str = r#"
        # rusticate:module(postgres):start
        [database]
        url = "postgres://app_rw@localhost/db"
        # rusticate:module(postgres):end
        [auth]
        issuer = "iss"
        audience = "aud"
        hs256_secret = "0123456789abcdef0123456789abcdef"
    "#;

    #[test]
    fn local_config_with_defaults_loads() {
        let config = Config::from_figment(&Figment::from(Toml::string(BASE))).unwrap();

        assert_eq!(config.environment, Environment::Local);
        assert_eq!(config.http.bind.port(), 8080);
    }

    #[test]
    fn production_rejects_shared_secret_tokens_and_docs_ui() {
        let figment = Figment::from(Toml::string(BASE)).merge(Toml::string(
            "environment = \"production\"\n[http]\ndocs_ui = true",
        ));

        let err = Config::from_figment(&figment).unwrap_err().to_string();

        assert!(err.contains("hs256_secret"));
        assert!(err.contains("docs_ui"));
    }

    #[test]
    fn unknown_keys_are_rejected() {
        let figment = Figment::from(Toml::string(BASE)).merge(Toml::string("databse_typo = 1"));

        assert!(Config::from_figment(&figment).is_err());
    }
}
