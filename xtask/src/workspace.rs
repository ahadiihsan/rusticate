//! Workspace model: members, their tier, and `architecture.toml`.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use anyhow::{Context as _, Result};
use serde::Deserialize;

use crate::metadata::{self, Metadata, Package};

pub fn root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .map_or_else(|| PathBuf::from("."), Path::to_path_buf)
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Architecture {
    pub tiers: Vec<Tier>,
    pub domains: Domains,
    pub capabilities: BTreeMap<String, Capability>,
    #[serde(default)]
    pub forbidden_features: Vec<ForbiddenFeature>,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Tier {
    pub name: String,
    pub members: Vec<String>,
    #[serde(default)]
    pub may_depend_on: Vec<String>,
    #[serde(default)]
    pub exempt: bool,
    #[serde(default)]
    pub dev_only: bool,
    pub allowed_external: Option<Vec<String>>,
    #[serde(default)]
    pub allowed_dev_external: Vec<String>,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Domains {
    pub shared: Vec<String>,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Capability {
    pub rule: String,
    pub description: String,
    pub allowed: Vec<String>,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ForbiddenFeature {
    pub rule: String,
    pub feature: String,
    pub in_tiers: Vec<String>,
}

/// A workspace member with its tier.
#[derive(Debug, Clone)]
pub struct Member {
    pub name: String,
    /// Directory relative to the workspace root, e.g. `crates/domains/billing`.
    pub dir: String,
    pub abs_dir: PathBuf,
    pub tier: String,
    pub exempt: bool,
}

impl Member {
    pub fn manifest(&self) -> PathBuf {
        self.abs_dir.join("Cargo.toml")
    }

    pub fn has_migrations(&self) -> bool {
        self.abs_dir.join("migrations").is_dir()
    }
}

pub struct Workspace {
    pub metadata: Metadata,
    pub architecture: Architecture,
    pub members: Vec<Member>,
}

impl Workspace {
    pub fn load() -> Result<Self> {
        let root = root();
        let architecture: Architecture = toml::from_str(
            &std::fs::read_to_string(root.join("architecture.toml"))
                .context("reading architecture.toml")?,
        )
        .context("parsing architecture.toml")?;
        let metadata = metadata::load(&root.join("Cargo.toml"))?;

        let mut members = Vec::new();
        for package in &metadata.packages {
            let abs_dir = package
                .manifest_path
                .parent()
                .map(Path::to_path_buf)
                .context("manifest without parent")?;
            let dir = abs_dir
                .strip_prefix(&root)
                .unwrap_or(&abs_dir)
                .to_string_lossy()
                .replace('\\', "/");
            let tier = architecture
                .tiers
                .iter()
                .find(|tier| tier.members.iter().any(|glob| glob_match(glob, &dir)))
                .with_context(|| format!("{dir} matches no tier in architecture.toml"))?;
            members.push(Member {
                name: package.name.clone(),
                dir,
                abs_dir,
                tier: tier.name.clone(),
                exempt: tier.exempt,
            });
        }
        members.sort_by(|a, b| a.dir.cmp(&b.dir));
        Ok(Self {
            metadata,
            architecture,
            members,
        })
    }

    pub fn member(&self, name: &str) -> Option<&Member> {
        self.members.iter().find(|member| member.name == name)
    }

    pub fn package(&self, name: &str) -> Option<&Package> {
        self.metadata
            .packages
            .iter()
            .find(|package| package.name == name)
    }

    pub fn tier(&self, name: &str) -> Option<&Tier> {
        self.architecture
            .tiers
            .iter()
            .find(|tier| tier.name == name)
    }

    /// Crates that own a schema, in migration order: foundation first.
    pub fn schema_owners(&self) -> Vec<&Member> {
        let mut owners: Vec<&Member> = self.members.iter().filter(|m| m.has_migrations()).collect();
        owners.sort_by_key(|m| (m.tier != "foundation", m.dir.clone()));
        owners
    }
}

/// Minimal path glob: `*` matches within one path segment.
pub fn glob_match(pattern: &str, path: &str) -> bool {
    let pattern: Vec<&str> = pattern.split('/').collect();
    let path: Vec<&str> = path.split('/').collect();
    pattern.len() == path.len() && pattern.iter().zip(&path).all(|(p, s)| segment_match(p, s))
}

fn segment_match(pattern: &str, segment: &str) -> bool {
    match pattern.split_once('*') {
        None => pattern == segment,
        Some((prefix, suffix)) => {
            segment.len() >= prefix.len() + suffix.len()
                && segment.starts_with(prefix)
                && segment.ends_with(suffix)
        }
    }
}

/// `schema` of a schema-owning crate, from its `sqlx.toml`.
pub fn owned_schema(member: &Member) -> Result<Option<String>> {
    let path = member.abs_dir.join("sqlx.toml");
    if !path.exists() {
        return Ok(None);
    }
    let value: toml::Value = toml::from_str(&std::fs::read_to_string(&path)?)?;
    Ok(value
        .get("migrate")
        .and_then(|m| m.get("create-schemas"))
        .and_then(toml::Value::as_array)
        .and_then(|schemas| schemas.first())
        .and_then(toml::Value::as_str)
        .map(str::to_owned))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn globs_match_single_segments() {
        assert!(glob_match(
            "crates/domains/*-engine",
            "crates/domains/billing-engine"
        ));
        assert!(!glob_match(
            "crates/domains/*-engine",
            "crates/domains/billing"
        ));
        assert!(glob_match("crates/domains/*", "crates/domains/billing"));
        assert!(!glob_match("apps/*", "apps/api/nested"));
    }
}
