# 0003. Upgrade a project's tooling by three-way merge

- **Status:** Accepted
- **Date:** 2026-10-16
- **Deciders:** Platform team
- **Rules affected:** none (tooling)

## Context

Improvements to the template (checks, scaffolds, skills, CI) reached only new projects. Existing projects drifted from the standard they were generated with.

## Decision

1. Every generated project records its origin in `rusticate.toml`: template source, exact revision, modules, name and owner.
2. `cargo rusticate upgrade [--to <ref>]` generates the template at the recorded revision and at the new one, with the project's identity and modules, and merges the difference into the project three ways (`git merge-file`). Conflicts are left as standard conflict markers.
3. Scope: tooling and the agent harness only (`xtask/`, `.agents/`, `.claude/`, `.gemini/`, `.github/`, `.githooks/`, `.config/`, `.cargo/`, `scripts/`, `docs/sop/`, configuration and operations docs, `AGENTS.md`, `CLAUDE.md`, the root `Cargo.toml`, lint configs). Product code (`apps/`, `crates/`, migrations) and the project's own ADRs are never touched. Symlinks are left alone.
4. The working tree must be clean, so the result is reviewed and undone with git.

## Alternatives considered

| Option | Why not |
|---|---|
| Also merge the platform crates | They are often customised; more conflicts than value for now. Can be added as an opt-in scope. |
| Report the diff only | Leaves the real work to people; agents cannot apply it reliably. |

## Consequences

Projects generated before cargo-rusticate 0.3.1 have no `rusticate.toml` and cannot be upgraded automatically. Changes to product code in the template (platform crates, the example) still need manual porting.
