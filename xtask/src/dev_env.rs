//! Local development configuration: the single source of the defaults that
//! `docker-compose.yml` provisions. Applied to child processes only when the
//! variable is not already set, so CI (or a developer) can override any of it.
//! There is deliberately no `.env` file.
//!
//! Defaults are per binary: each binary's config rejects unknown `APP_*`
//! keys (CFG-002), so the api must never see the worker's settings and
//! vice versa.

use std::process::Command;

// rusticate:module(postgres):start
pub const DATABASE_URL: &str = "postgres://app_owner:app_owner@localhost:55432/app";
// rusticate:module(postgres):end
pub const ISSUER: &str = "https://auth.local.test";
pub const AUDIENCE: &str = "rusticate-api";
/// Development-only signing secret; production verifies with a public key.
pub const HS256_SECRET: &str = "local-dev-secret-at-least-32-bytes-long!";

// rusticate:module(postgres):start
const APP_RW_URL: &str = "postgres://app_rw:app_rw@localhost:55432/app";
// rusticate:module(postgres):end
// rusticate:module(messaging):start
const APP_SYSTEM_URL: &str = "postgres://app_system:app_system@localhost:55432/app";
// rusticate:module(messaging):end

/// Tooling defaults: migrations, query metadata, `#[sqlx::test]`.
const TOOLING: &[(&str, &str)] = &[
    // rusticate:module(postgres):start
    ("DATABASE_URL", DATABASE_URL),
    // rusticate:module(postgres):end
    ("RUST_LOG", "info,sqlx=warn"),
];

const API: &[(&str, &str)] = &[
    ("APP_ENVIRONMENT", "local"),
    // rusticate:module(postgres):start
    ("APP_DATABASE__URL", APP_RW_URL),
    ("APP_MIGRATION_DATABASE_URL", DATABASE_URL),
    // rusticate:module(postgres):end
    ("APP_AUTH__ISSUER", ISSUER),
    ("APP_AUTH__AUDIENCE", AUDIENCE),
    ("APP_AUTH__HS256_SECRET", HS256_SECRET),
    ("APP_HTTP__DOCS_UI", "true"),
    ("APP_TELEMETRY__LOG_FORMAT", "pretty"),
];

// rusticate:module(messaging):start
const WORKER: &[(&str, &str)] = &[
    ("APP_DATABASE__URL", APP_RW_URL),
    ("APP_SYSTEM_DATABASE__URL", APP_SYSTEM_URL),
    ("APP_NATS__URL", "nats://localhost:4222"),
    ("APP_TELEMETRY__LOG_FORMAT", "pretty"),
];
// rusticate:module(messaging):end

/// The binaries `cargo rusticate run` can start.
#[derive(Debug, Clone, Copy, clap::ValueEnum)]
pub enum App {
    Api,
    // rusticate:module(messaging):start
    Worker,
    // rusticate:module(messaging):end
}

impl App {
    pub const fn package(self) -> &'static str {
        match self {
            Self::Api => "api",
            // rusticate:module(messaging):start
            Self::Worker => "worker",
            // rusticate:module(messaging):end
        }
    }
}

/// Adds the tooling defaults the caller has not overridden.
pub fn apply(command: &mut Command) {
    set_missing(command, TOOLING);
}

/// Adds the defaults of one binary (and the tooling ones).
pub fn apply_for(command: &mut Command, app: App) {
    set_missing(command, TOOLING);
    set_missing(
        command,
        match app {
            App::Api => API,
            // rusticate:module(messaging):start
            App::Worker => WORKER,
            // rusticate:module(messaging):end
        },
    );
}

fn set_missing(command: &mut Command, defaults: &[(&str, &str)]) {
    for (key, value) in defaults {
        if !is_set(key) {
            command.env(key, value);
        }
    }
}

#[expect(
    clippy::disallowed_methods,
    reason = "xtask is the one place that reads the developer's environment"
)]
fn is_set(key: &str) -> bool {
    std::env::var_os(key).is_some()
}
