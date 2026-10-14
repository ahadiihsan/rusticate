//! Validated names: the new project and the GitHub account that owns it.

use anyhow::{Result, ensure};

/// Derived Kubernetes names (`<name>-migrate`) must fit a 63-character DNS
/// label; 40 leaves room for every suffix the template uses.
const MAX_NAME_LEN: usize = 40;

/// GitHub logins are at most 39 characters.
const MAX_OWNER_LEN: usize = 39;

/// The project name, in kebab case (`my-service`).
///
/// It is used verbatim for the compose project, image names, Kubernetes
/// resources and the token audience, so it must be a DNS label that starts
/// with a letter.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct ProjectName(String);

impl ProjectName {
    pub(crate) fn parse(raw: &str) -> Result<Self> {
        let starts_with_letter = raw.bytes().next().is_some_and(|b| b.is_ascii_lowercase());
        ensure!(
            starts_with_letter && is_kebab(raw, MAX_NAME_LEN, |b| b.is_ascii_lowercase()),
            "invalid project name `{raw}`: use lowercase letters, digits and single hyphens, \
             starting with a letter, at most {MAX_NAME_LEN} characters (e.g. `my-service`)"
        );
        Ok(Self(raw.to_owned()))
    }

    pub(crate) fn as_str(&self) -> &str {
        &self.0
    }

    /// Title case for prose and titles: `my-service` → `My Service`.
    pub(crate) fn title(&self) -> String {
        self.0
            .split('-')
            .map(|word| {
                let mut chars = word.chars();
                chars.next().map_or_else(String::new, |first| {
                    first.to_ascii_uppercase().to_string() + chars.as_str()
                })
            })
            .collect::<Vec<_>>()
            .join(" ")
    }
}

/// The GitHub user or organisation that will own the repository.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct Owner(String);

impl Owner {
    /// Accepts `acme` or `@acme`.
    pub(crate) fn parse(raw: &str) -> Result<Self> {
        let login = raw.strip_prefix('@').unwrap_or(raw);
        ensure!(
            is_kebab(login, MAX_OWNER_LEN, |b| b.is_ascii_alphabetic()),
            "invalid GitHub owner `{raw}`: use a user or organisation login, e.g. `acme`"
        );
        Ok(Self(login.to_owned()))
    }

    pub(crate) fn as_str(&self) -> &str {
        &self.0
    }
}

/// Letters (as `letter` defines them), digits and single inner hyphens.
fn is_kebab(raw: &str, max_len: usize, letter: impl Fn(u8) -> bool) -> bool {
    !raw.is_empty()
        && raw.len() <= max_len
        && raw
            .bytes()
            .all(|b| letter(b) || b.is_ascii_digit() || b == b'-')
        && !raw.starts_with('-')
        && !raw.ends_with('-')
        && !raw.contains("--")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn accepts_dns_label_names() {
        for name in [
            "billing",
            "my-service",
            "a1",
            "x-2-y",
            &"a".repeat(MAX_NAME_LEN),
        ] {
            assert!(ProjectName::parse(name).is_ok(), "{name}");
        }
    }

    #[test]
    fn rejects_names_that_break_dns_or_postgres_rules() {
        for name in [
            "",
            "My-Service",
            "my_service",
            "-svc",
            "svc-",
            "my--svc",
            "1svc",
            "svc.io",
            &"a".repeat(MAX_NAME_LEN + 1),
        ] {
            let err = ProjectName::parse(name).unwrap_err().to_string();
            assert!(err.contains("invalid project name"), "{name}: {err}");
        }
    }

    #[test]
    fn title_capitalises_each_word() {
        let name = ProjectName::parse("my-ledger-2").unwrap();
        assert_eq!(name.title(), "My Ledger 2");
        assert_eq!(ProjectName::parse("api").unwrap().title(), "Api");
    }

    #[test]
    fn owner_accepts_logins_with_or_without_at() {
        assert_eq!(Owner::parse("@Acme-Corp").unwrap().as_str(), "Acme-Corp");
        assert_eq!(Owner::parse("dev42").unwrap().as_str(), "dev42");
    }

    #[test]
    fn owner_rejects_non_logins() {
        for owner in [
            "",
            "acme/platform",
            "-acme",
            "acme--x",
            "a b",
            &"a".repeat(40),
        ] {
            assert!(Owner::parse(owner).is_err(), "{owner}");
        }
    }
}
