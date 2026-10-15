//! Checks for `cargo-rusticate`, the project generator that lives next to the
//! template (`cargo rusticate new`).
//!
//! It is a standalone crate outside the workspace, and generated projects do
//! not contain it: there, `present()` is false and every check is skipped.

use std::path::PathBuf;
use std::process::Command;

use anyhow::{Context as _, Result};

use crate::{metadata, sh, workspace};

const DIR: &str = "cargo-rusticate";

/// The project the smoke test generates. Its name must not contain the
/// template's, so any identity left behind would be noticed.
const SMOKE_PROJECT: &str = "smoke-service";

fn manifest() -> PathBuf {
    workspace::root().join(DIR).join("Cargo.toml")
}

pub fn present() -> bool {
    manifest().is_file()
}

/// `cargo <command> --manifest-path cargo-rusticate/Cargo.toml <args>`.
fn cargo(command: &[&str], args: &[&str]) -> Command {
    let mut cmd = sh::cargo(command);
    cmd.arg("--manifest-path").arg(manifest()).args(args);
    cmd
}

pub fn fmt(check: bool) -> Result<()> {
    sh::run(cargo(&["fmt"], if check { &["--check"] } else { &[] }))
}

/// Format, lint and test the generator.
pub fn check() -> Result<()> {
    fmt(true)?;
    sh::run(cargo(
        &["clippy"],
        &["--all-targets", "--locked", "--", "-D", "warnings"],
    ))?;
    sh::run(cargo(&["test"], &["--locked"]))
}

/// Advisories and licenses of the generator's dependencies (it ships to crates.io).
pub fn deny() -> Result<()> {
    let mut cmd = cargo(&["deny"], &["--config"]);
    cmd.arg(workspace::root().join("deny.toml")).arg("check");
    sh::run(cmd)
}

/// The generator builds with the `rust-version` it declares (installs that
/// toolchain through rustup on first use).
pub fn msrv() -> Result<()> {
    let metadata = metadata::load(&manifest()).context("reading cargo-rusticate's manifest")?;
    let rust_version = metadata
        .packages
        .first()
        .and_then(|package| package.rust_version.clone())
        .context("cargo-rusticate must declare a rust-version")?;
    sh::run(sh::cmd(
        "rustup",
        [
            "toolchain",
            "install",
            rust_version.as_str(),
            "--profile",
            "minimal",
            "--no-self-update",
        ],
    ))?;
    sh::run(cargo(
        &[&format!("+{rust_version}"), "check"],
        &["--locked"],
    ))
}

/// Generates a project from this checkout and runs its own `verify --fast`:
/// proof that what users get builds, lints and passes the architecture check.
pub fn smoke() -> Result<()> {
    let target = workspace::root().join("target");
    let out = target.join("generator-smoke");
    if out.exists() {
        std::fs::remove_dir_all(&out).with_context(|| format!("removing {}", out.display()))?;
    }
    std::fs::create_dir_all(&out).with_context(|| format!("creating {}", out.display()))?;
    let project = out.join(SMOKE_PROJECT);

    let mut generate = cargo(&["run", "--quiet"], &["--locked", "--"]);
    generate
        .args(["new", SMOKE_PROJECT, "--owner", "smoke-owner", "--template"])
        .arg(workspace::root())
        .arg("--path")
        .arg(&project);
    sh::run(generate)?;

    let mut verify = sh::cargo(["xtask", "verify", "--fast"]);
    // A separate target dir: same-named packages from two workspaces must
    // not overwrite each other's binaries.
    verify
        .current_dir(&project)
        .env("CARGO_TARGET_DIR", target.join("smoke-build"));
    sh::run(verify)
}
