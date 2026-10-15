//! The few fields of `cargo metadata` xtask reads.
//!
//! Parsed directly rather than through the `cargo_metadata` crate: that crate
//! enables `serde_json/unbounded_depth`, and because xtask is a workspace
//! member, every workspace build (tests, clippy, rust-analyzer) then compiled
//! `serde_json` and everything above it (axum, sqlx, async-nats, ...) with a
//! different feature set than `cargo run -p api`, i.e. twice.

use std::path::{Path, PathBuf};
use std::process::Command;

use anyhow::{Context as _, Result, ensure};
use serde::Deserialize;

#[derive(Debug, Deserialize)]
pub struct Metadata {
    /// With `--no-deps`: exactly the workspace members.
    pub packages: Vec<Package>,
}

#[derive(Debug, Deserialize)]
pub struct Package {
    pub name: String,
    pub manifest_path: PathBuf,
    pub dependencies: Vec<Dependency>,
    pub rust_version: Option<String>,
}

#[derive(Debug, Deserialize)]
pub struct Dependency {
    pub name: String,
    /// `None` for normal dependencies, `"dev"` or `"build"` otherwise.
    pub kind: Option<String>,
    #[serde(default)]
    pub features: Vec<String>,
}

impl Dependency {
    pub fn is_dev(&self) -> bool {
        self.kind.as_deref() == Some("dev")
    }
}

/// `cargo metadata --no-deps` for the manifest at `manifest_path`.
pub fn load(manifest_path: &Path) -> Result<Metadata> {
    let output = Command::new("cargo")
        .args([
            "metadata",
            "--format-version",
            "1",
            "--no-deps",
            "--manifest-path",
        ])
        .arg(manifest_path)
        .output()
        .context("running cargo metadata")?;
    ensure!(
        output.status.success(),
        "cargo metadata failed: {}",
        String::from_utf8_lossy(&output.stderr).trim()
    );
    serde_json::from_slice(&output.stdout).context("parsing cargo metadata")
}
