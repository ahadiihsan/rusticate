//! Dependency-graph rules (ARCH-001..ARCH-003, ENG-001, SEC-005).

use super::Violation;
use crate::metadata::Dependency;
use crate::workspace::{Member, Tier, Workspace};

pub fn check(workspace: &Workspace) -> Vec<Violation> {
    let mut violations = Vec::new();
    for member in workspace.members.iter().filter(|m| !m.exempt) {
        let (Some(package), Some(tier)) = (
            workspace.package(&member.name),
            workspace.tier(&member.tier),
        ) else {
            continue;
        };
        for dependency in &package.dependencies {
            if dependency.name == member.name {
                continue; // self dev-dependency used to enable test features
            }
            let is_dev = dependency.is_dev();
            match workspace.member(&dependency.name) {
                Some(target) => internal(workspace, member, tier, target, is_dev, &mut violations),
                None => external(member, tier, dependency, is_dev, &mut violations),
            }
            forbidden_features(workspace, member, dependency, is_dev, &mut violations);
        }
    }
    violations
}

fn internal(
    workspace: &Workspace,
    member: &Member,
    tier: &Tier,
    target: &Member,
    is_dev: bool,
    violations: &mut Vec<Violation>,
) {
    let location = format!("{}/Cargo.toml → {}", member.dir, target.name);
    if target.exempt {
        violations.push(Violation::new(
            "ARCH-001",
            location,
            "nothing may depend on tooling crates",
        ));
        return;
    }
    let target_dev_only = workspace.tier(&target.tier).is_some_and(|t| t.dev_only);
    if target_dev_only {
        if !is_dev {
            violations.push(Violation::new(
                "ARCH-003",
                location,
                format!(
                    "`{}` is test-only: move it to [dev-dependencies]",
                    target.name
                ),
            ));
        }
        return;
    }
    let allowed = tier.may_depend_on.iter().any(|rule| match rule.as_str() {
        "own-engine" => target.name == format!("{}-engine", member.name),
        "shared-domain" => {
            target.tier == "domain" && workspace.architecture.domains.shared.contains(&target.name)
        }
        tier_name => target.tier == tier_name,
    });
    if !allowed {
        let hint = if member.tier == "domain" && target.tier == "domain" {
            " Contexts communicate through events (MSG-*), or the target must be listed in [domains].shared."
        } else {
            ""
        };
        violations.push(Violation::new(
            "ARCH-002",
            location,
            format!(
                "tier `{}` may not depend on `{}` (tier `{}`).{hint}",
                member.tier, target.name, target.tier
            ),
        ));
    }
}

fn external(
    member: &Member,
    tier: &Tier,
    dependency: &Dependency,
    is_dev: bool,
    violations: &mut Vec<Violation>,
) {
    let Some(allowed) = &tier.allowed_external else {
        return;
    };
    let permitted = allowed.contains(&dependency.name)
        || (is_dev && tier.allowed_dev_external.contains(&dependency.name));
    if !permitted {
        violations.push(Violation::new(
            "ENG-001",
            format!("{}/Cargo.toml → {}", member.dir, dependency.name),
            format!(
                "`{}` is not on the {} allowlist in architecture.toml (engines are pure: no I/O, no async)",
                dependency.name, member.tier
            ),
        ));
    }
}

fn forbidden_features(
    workspace: &Workspace,
    member: &Member,
    dependency: &Dependency,
    is_dev: bool,
    violations: &mut Vec<Violation>,
) {
    if is_dev {
        return;
    }
    for forbidden in &workspace.architecture.forbidden_features {
        let Some((crate_name, feature)) = forbidden.feature.split_once('/') else {
            continue;
        };
        if forbidden.in_tiers.contains(&member.tier)
            && dependency.name == crate_name
            && dependency.features.iter().any(|f| f == feature)
        {
            violations.push(Violation::new(
                forbidden.rule.clone(),
                format!("{}/Cargo.toml → {}", member.dir, dependency.name),
                format!(
                    "feature `{}` must never reach a deployable binary",
                    forbidden.feature
                ),
            ));
        }
    }
}
