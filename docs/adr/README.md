# Architecture Decision Records

An ADR records one decision that changes how the system is built: a new bounded context, a change to `architecture.toml`, a lint or allowlist change, a rule exception, or a new platform dependency (GOV-001).

- File: `NNNN-short-title.md`, numbered in order and never renumbered.
- Copy [0000-template.md](0000-template.md).
- Status moves `Proposed → Accepted → (Superseded by NNNN | Deprecated)`. Accepted ADRs are not edited; supersede them.
- The PR that implements the decision links the ADR. The platform owners review it (CODEOWNERS).

| ADR | Decision | Status |
|---|---|---|
| [0001](0001-adopt-engineering-standard-v2.md) | Adopt the v2 engineering standard and its enforcement | Accepted |
