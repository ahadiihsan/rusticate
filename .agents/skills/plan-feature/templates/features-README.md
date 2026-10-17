# Features

<!-- Introduction, written by the team: what this service is, for whom, and why it is built on this boilerplate. Include:
     - **Big picture:** what it becomes.
     - **Users:** the roles.
     - **Now:** the features planned one at a time (the index below).
     - **Later:** known future needs, not planned. Designs must not block them, but build nothing for them.
     - **Not building:** explicit non-goals.
     - **Constraints:** scale, privacy rules, integrations. -->


We do not plan every feature at once. Each feature is planned, reviewed and built on its own, in a folder of this directory, following the `plan-feature` skill (`.agents/skills/plan-feature/SKILL.md`).

## What counts as a feature

A feature is one outcome a user gets, stated in one sentence ("Marketers can merge duplicate customer profiles"). It is useful on its own, writes to one bounded context, and is built as 1 to 6 slices: `verb_noun` use cases, one pull request each. A layer ("database schema"), a piece of technology, or bare CRUD is not a feature. Ideas not yet planned are listed as `proposed` rows in the index, with no folder.

## How a feature is planned

| Phase | Output | Gate |
|---|---|---|
| 1. Frame | `brief.md`: problem, stories, out of scope | stories and scope approved |
| 2. Specify | `spec.md`: glossary, rules, scenarios with IDs | no open questions, every rule has a scenario |
| 3. Design | `design.md`, `decisions/`: slices, traceability, build order, diagrams where needed | every scenario maps to a slice and a test |
| 4. Break down | `tickets/`: one pull request each, with dependencies and state | every slice, rule and scenario has a ticket |
| 5. Build | one pull request per ticket (`implement-feature`) | `cargo rusticate verify`; the ticket is `done` when its pull request merges |

Each document starts with a `status:` line (`draft` or `approved`). No code is written for a feature until its tickets are approved. Tickets follow the format in `.agents/skills/plan-feature/tickets.md`: Markdown with strict YAML front matter, with state `open`, `in_progress` or `done`.

## Layout

```text
features/
├── README.md              this file and the index below
└── NNN-<slug>/
    ├── brief.md
    ├── spec.md
    ├── design.md
    ├── decisions/NNNN-<slug>.md
    └── tickets/<KEY>-TNN.md
```

Decisions that only affect one feature live in its `decisions/`. Those that cross contexts or change a rule are project ADRs in `docs/adr/`.

## Index

| # | Feature | Scenario key | Status | Depends on |
|---|---|---|---|---|
<!-- | 001 | [Example](001-example/brief.md) | EXM | proposed · framing · specifying · designing · ready · building · done | - | -->
