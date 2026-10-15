//! The project's record of where it came from (`rusticate.toml`): the
//! template, its exact revision and the modules chosen. It is what a later
//! `cargo rusticate upgrade` compares against, so projects keep it committed.

use std::path::Path;

use anyhow::{Context as _, Result};
use serde::Serialize;

pub(crate) const RECORD: &str = "rusticate.toml";

#[derive(Debug, Serialize)]
struct Record<'a> {
    template: Template<'a>,
    project: Project<'a>,
}

#[derive(Debug, Serialize)]
struct Template<'a> {
    /// The git URL, or the local checkout, the project was generated from.
    source: &'a str,
    /// The tag, branch or commit of that source.
    #[serde(skip_serializing_if = "Option::is_none")]
    revision: Option<&'a str>,
    /// The optional modules included (core is always included).
    modules: Vec<&'a str>,
}

#[derive(Debug, Serialize)]
struct Project<'a> {
    name: &'a str,
    #[serde(skip_serializing_if = "Option::is_none")]
    owner: Option<&'a str>,
}

#[derive(Debug)]
pub(crate) struct Origin<'a> {
    pub(crate) source: &'a str,
    pub(crate) revision: Option<&'a str>,
    pub(crate) modules: Vec<&'a str>,
    pub(crate) name: &'a str,
    pub(crate) owner: Option<&'a str>,
}

/// Writes `rusticate.toml` at the project root.
pub(crate) fn write(dest: &Path, origin: &Origin<'_>) -> Result<()> {
    let record = Record {
        template: Template {
            source: origin.source,
            revision: origin.revision,
            modules: origin.modules.clone(),
        },
        project: Project {
            name: origin.name,
            owner: origin.owner,
        },
    };
    let body = toml::to_string(&record).context("serializing the project record")?;
    let text = format!(
        "# Where this project came from, written by cargo-rusticate. Keep it\n\
         # committed: upgrading to a newer template compares against it.\n\n{body}"
    );
    std::fs::write(dest.join(RECORD), text).with_context(|| format!("writing {RECORD}"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn records_the_template_version_modules_and_project() {
        let dir =
            std::env::temp_dir().join(format!("cargo-rusticate-record-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let origin = Origin {
            source: "https://github.com/acme/template.git",
            revision: Some("v1.2.3"),
            modules: vec!["postgres"],
            name: "svc",
            owner: None,
        };

        write(&dir, &origin).unwrap();

        let text = std::fs::read_to_string(dir.join(RECORD)).unwrap();
        let parsed: toml::Table = toml::from_str(&text).unwrap();
        assert_eq!(parsed["template"]["revision"].as_str(), Some("v1.2.3"));
        assert_eq!(parsed["template"]["modules"].as_array().unwrap().len(), 1);
        assert_eq!(parsed["project"]["name"].as_str(), Some("svc"));
        assert!(parsed["project"].get("owner").is_none());
        std::fs::remove_dir_all(&dir).unwrap();
    }
}
