//! `cargo xtask arch`: architecture conformance (SOP §11, rule catalogue).
//!
//! Three families of checks, all driven by `architecture.toml` and the
//! workspace itself:
//! * [`deps`]: the crate dependency graph (tiers, allowlists, features);
//! * [`source`]: what the code does (capabilities, purity, SQL, DTOs), from
//!   the syntax tree, not from grep;
//! * [`layout`]: how crates and feature slices are organised, migrations,
//!   lint wiring, router and test coverage.

mod deps;
mod layout;
mod source;

use anyhow::{Result, bail};

use crate::workspace::Workspace;

/// One broken rule at one place.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord)]
pub struct Violation {
    pub rule: String,
    pub location: String,
    pub message: String,
}

impl Violation {
    pub fn new(
        rule: impl Into<String>,
        location: impl Into<String>,
        message: impl Into<String>,
    ) -> Self {
        Self {
            rule: rule.into(),
            location: location.into(),
            message: message.into(),
        }
    }
}

pub fn run() -> Result<()> {
    let workspace = Workspace::load()?;
    let mut violations = deps::check(&workspace);
    violations.extend(source::check(&workspace)?);
    violations.extend(layout::check(&workspace)?);
    violations.sort();
    violations.dedup();

    let checked = workspace.members.iter().filter(|m| !m.exempt).count();
    if violations.is_empty() {
        println!("✓ architecture: {checked} crates conform");
        return Ok(());
    }
    for violation in &violations {
        eprintln!(
            "  [{}] {}\n      {}",
            violation.rule, violation.location, violation.message
        );
    }
    bail!(
        "{} architecture violation(s). Each [RULE-ID] is explained in docs/sop/rust-backend-sop-v2.md",
        violations.len()
    )
}
