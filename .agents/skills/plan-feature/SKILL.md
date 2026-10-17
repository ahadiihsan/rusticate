---
name: plan-feature
description: Plan a feature before any code is written, one phase per session with the human approving each gate - brief (why, stories, scope), spec (rules, example scenarios with IDs, open questions), design (slices mapped onto the boilerplate), tickets (one pull request each, with dependencies and state). Produces features/<NNN-slug>/ and hands off to implement-feature. Use when starting a new feature, when the user brings a PRD, idea or ticket that is not yet sliced, or when asked to plan, spec or design a feature.
---

# Plan a feature

Features are planned one at a time, in documents under `features/`, before any code. Planning everything at once overwhelms the agent and invites over-engineering; planning one feature, in small reviewed steps, keeps every decision small and visible.

## What a feature is

A **feature** (`features/<NNN-slug>/`) is one outcome a user gets, stated in one sentence with a user as the subject: "Marketers can merge duplicate customer profiles." It is what this skill plans. It must pass all four checks:

| Check | Too big if | Too small if |
|---|---|---|
| **One outcome**: the sentence has one verb a user cares about | it needs "and" ("ingest events *and* build segments") | it states a technical step ("add a profile table") |
| **Useful alone**: once shipped it is worth having, even if no later feature is ever built | | it only matters once a later feature exists |
| **One owning context**: its writes go to a single bounded context (it may read or react to others) | it writes to two contexts | |
| **1 to 6 slices** | more than six | one slice with no business rule: build it with [implement-feature](../implement-feature/SKILL.md) directly, with no plan |

Never a feature: a layer or a piece of technology ("database schema", "auth middleware", "Kafka setup"), or bare CRUD ("manage customers"). Ask what the user achieves with it, and plan that instead.

A **slice** is one `verb_noun` use case (`merge_profiles`), one pull request. It is what `cargo rusticate new feature` scaffolds and [implement-feature](../implement-feature/SKILL.md) builds, even though the CLI calls it a feature. A feature becomes one or more slices.

Run the four checks in Phase 1 once the sentence is written, and again in Phase 3 once the slices are known. If any check fails, handle it as below before going on.

### When the request is not one outcome

Never plan a request that fails the checks, and never pick the outcome for the human. Show them the problem and ask them to choose.

| The human gives | Do |
|---|---|
| **Several outcomes** (a PRD, a roadmap, "X and Y and Z") | Split it into one-sentence candidates. Propose an order: dependencies first, then the smallest one that is useful. Ask which one to plan now. The others become `proposed` rows in the index, with no folder. |
| **No outcome** ("we need customer data", "something like Segment") | Ask: *who* uses it, and what can they do afterwards that they cannot do today? Offer two or three candidate sentences to choose from or correct. |
| **A technical task** ("auth", "set up Kafka", "profile table") | Ask which user outcome needs it, and plan that outcome; the task becomes one of its slices. If the work has no user outcome (platform, tooling, upgrades), it is not a feature: say so, and suggest [implement-feature](../implement-feature/SKILL.md) or an ADR. |
| **One outcome, but it fails another check** | Too many slices or two contexts: propose a split along the slices or contexts. Too small: suggest building it directly. Not useful alone: merge it into the feature it serves. |

Stop after the proposal and wait for the human to choose. If the human insists on a scope that fails a check, record it under *Risks* in `design.md` with their reason, and go on.

## Ground rules

- **Docs only.** This skill writes Markdown under `features/` and nothing else: no code, migrations or scaffolds. Building starts with [implement-feature](../implement-feature/SKILL.md), slice by slice.
- **One phase per session, then stop.** End each phase by asking the human to review. Move on only when they approve, and record it by setting `status: approved` at the top of the file.
- **Ask, never invent.** When the human has not said something (a rule, a limit, who may do what), add it to *Open questions*. A guessed requirement is the main source of over-engineering.
- **Small and concrete.** Use the domain's words, and cut whatever the stories do not need.
- **Read only what the phase needs:** `features/README.md`, this feature's folder, the features it names as dependencies, and for the design phase AGENTS.md and the owning context's code.

## Phase 0: find or create the folder

1. If `features/README.md` does not exist, create it from [templates/features-README.md](templates/features-README.md) and ask the human to write the introduction.
2. Check the index in `features/README.md`. If the feature already has a folder, work out which phase it is in: it is the first file whose `status:` is not `approved`.
3. A `proposed` row in the index has no folder yet; it becomes a feature when the human picks it.
4. For a new feature, pick the next number and a short kebab-case slug (`004-consent`). Copy [templates/brief.md](templates/brief.md), [templates/spec.md](templates/spec.md) and [templates/design.md](templates/design.md) into the folder. Add a row to the index with status `framing`.

## Phase 1: Frame (`brief.md`)

Interview the human, a few questions at a time:

- The feature in one sentence, with a user as the subject. It goes at the top of `brief.md`. Then run [the four checks](#what-a-feature-is).
- What problem, for whom? Use their words. These words become the glossary.
- The user stories: `As a <role>, I want <goal>, so that <outcome>`.
- **Out of scope.** Ask directly what this feature will *not* do. Push things here: later features, admin tools, generalisations.
- Which earlier features this one depends on.
- How we will know it works (a success signal).

Minimum for `brief.md` (every item is required; write "none" rather than leave one empty):

- [ ] the one-sentence outcome, passing the four checks;
- [ ] the problem: who has it, and what it costs them today;
- [ ] at least one user story, and every story names a role that exists in the problem;
- [ ] at least one *out of scope* item;
- [ ] *depends on*: earlier features, or "none";
- [ ] a success signal someone could observe after shipping (a behaviour or a number, not "it works").

Gate: every item above is ticked, and the human approves the stories and the out-of-scope list.

## Phase 2: Specify (`spec.md`)

Use Example Mapping: **rules**, each illustrated by **examples**, plus **questions**.

1. **Glossary.** One line per domain term. These become the names of value objects, verbs and permissions later, so settle them now.
2. **Rules** `R1..Rn`: one business rule each, in plain language. Mark each rule *calculation / invariant* (it goes to `<ctx>-engine`) or *policy* (who may do it, or what the server decides).
3. **Scenarios** under each rule, in Given / When / Then, each with a stable ID `<KEY>-S<NN>` (`CONS-S04`). Cover the happy path, every way the rule rejects input, and the edge values (zero, maximum, duplicate, already done). IDs are never reused or renumbered; delete a scenario by marking it `(removed)`.
4. **Open questions**, each with an owner. Write them as questions the human can answer in one line.

Leave out the scenarios every slice gets anyway: missing permission, another tenant, idempotent replay, concurrency. The design adds those.

Minimum for `spec.md`:

- [ ] a scenario key (`CONS`), also written in the index;
- [ ] a glossary entry for every domain noun used in the rules and scenarios;
- [ ] at least one rule, each marked *calculation / invariant* or *policy*;
- [ ] every rule has a happy-path scenario, and every rule that can reject has a rejection scenario that names the reason;
- [ ] every user story in `brief.md` is covered by at least one scenario;
- [ ] no scenario mentions tables, endpoints, status codes or JSON;
- [ ] the open questions table is empty, or every row has an answer.

Gate: every item above is ticked, and the human approves.

## Phase 3: Design (`design.md`)

Map the approved spec onto this boilerplate. Read AGENTS.md and the [SOP](../../../docs/sop/rust-backend-sop-v2.md) sections you cite. Then:

1. **Owning context.** An existing context, or a new one ([new-bounded-context](../new-bounded-context/SKILL.md): that needs a project ADR in `docs/adr/`). Check `crates/domains/` first. For anything about identity, tokens or permissions, check what `iam` already provides.
2. **Slices.** Fill in the slice table with one row per `verb_noun`, using the classification questions in [implement-feature](../implement-feature/SKILL.md#1-classify-before-writing-code): kind, permission `<ctx>.<noun>.<verb>`, engine rules (by `R` ID), schema change, idempotency / `If-Match`, events, endpoint.
3. **Traceability.** Every scenario ID maps to exactly one slice and one test name, and every slice lists its standard tests (permission, tenant isolation, plus replay and concurrency where they apply). A scenario that no slice covers means the design is incomplete. A slice that covers no scenario is probably out of scope.
4. **Build order.** Order the slices by dependency, with the smallest end-to-end slice first.
5. **Decisions.** Write a decision in `decisions/NNNN-<slug>.md` ([template](templates/decision.md)) for a choice that has real alternatives and affects only this feature. A choice that crosses contexts, adds a context, or needs a rule exception (GOV-001) goes in `docs/adr/` instead; link it here. Any rule exception is stated with its rule ID, never hidden.
6. **Diagrams**, only where a table cannot show it. Draw them in Mermaid, inside `design.md`:

   | The design has | Draw |
   |---|---|
   | a lifecycle with three or more states | a state diagram: every transition is a rule and has a scenario |
   | a flow across contexts, events or async steps | a sequence diagram, including where it can fail and what retries |
   | a new context, or new dependencies between contexts | a context map, in the project ADR rather than here |
   | anything else | no diagram |

   A diagram uses only names from the glossary and the slice table. A state, step or participant that is not in the spec means the design invented something: fix the spec or the diagram.
7. **Evolution check.** Look for evidence that this feature will grow in a direction the design could block (see [below](#evolution-check)). Ask the human once, and record the answer under *Evolution*.

### Evolution check

**Evidence**, not hunches. Only these count:

- the introduction's *Later* list, or a `proposed` row in the index, extends this feature (email campaigns now, "LINE and other channels" later);
- the human said "for now", "first", "at least", "near future", or listed kinds ("a booking, an experience or goods");
- a glossary term has several kinds, and the spec covers only some of them;
- a dependency of a later feature reads data this feature owns (a future segment predicate over attributes stored here).

If there is no evidence, write "none found" under *Evolution* and do not ask. If there is, ask **one** question that lists each signal with two choices:

| Choice | What it means |
|---|---|
| **Prepare the shape** | Only what is cheap now and expensive later: data shape, keys, enums, and naming that leave room (consent per channel, not an `email_opt_in` flag; a `kind` column with today's values). For each item, list what it costs now and what it saves later. |
| **Today only** (always offered, never discouraged) | Design for the stories as written. The risk is recorded, and nothing else changes. |

Never offer generic *code*: plugin interfaces, provider abstractions, frameworks for kinds that do not exist yet. Generic code is extracted when the second real case is planned, and both cases shape it. If the human asks for it anyway, record it as a decision with its reason, and build the smallest version.

Record the answer for each signal under *Evolution* in `design.md`: the signal, the choice, and the items prepared, or the risk accepted. Do not ask again about a recorded signal. Raise it again only when new evidence appears (a new *Later* item, or a new story).

Minimum for `design.md`:

- [ ] the owning context, with one line on why (and the ADR link if it is new);
- [ ] at least one slice, with every column of the slice table filled in (`none` or `n/a` is an answer, a blank is not);
- [ ] every scenario ID appears in the traceability table exactly once, with a test name;
- [ ] every slice has its standard tests listed;
- [ ] a build order covering every slice;
- [ ] *decisions*: links, or "none";
- [ ] *risks*: at least one, or "none" with a reason;
- [ ] the diagrams the table in step 6 requires, or none when it requires none;
- [ ] *evolution*: each signal with the human's choice, or "none found".

Gate: every item above is ticked, the feature still passes [the four checks](#what-a-feature-is), and the human approves.

## Phase 4: Break down (`tickets/`)

Turn the approved design into tickets, in the format of [tickets.md](tickets.md): one file per ticket in `features/<NNN-slug>/tickets/`, each one pull request that passes `verify` on its own.

1. One `context` ticket if the design adds a context, and one `migration` ticket per schema change.
2. One `slice` ticket per slice in the slice table. Move shared or large engine rules into `engine` tickets.
3. Fill `depends_on` from the build order and the dependency column of [tickets.md](tickets.md#size). Fill `touches` with the crates and shared files each ticket changes.
4. Give each ticket its scenarios, rules and *Read first* links, so an agent can do it without reading the rest of the feature.

Minimum: the [checks in tickets.md](tickets.md#checks-before-the-tickets-are-approved).

Gate: every check is ticked and the human approves. Set the index status to `ready`.

## Hand-off

Building starts only when the human asks for it ("build <feature>" or "do CONS-T03"). Each ticket follows [implement-feature](../implement-feature/SKILL.md). Its state (`open`, `in_progress`, `done`) changes only as [tickets.md](tickets.md#state) allows, and the feature's status in the index follows its tickets.

Name integration tests after their scenarios (`cons_s04_withdrawn_consent_blocks_export`). The spec describes current behaviour: if building shows it was wrong, change `spec.md` in the same pull request, keep the scenario IDs, and tell the human.

## When needs change

New needs arrive while work is under way ("we also want LINE campaigns"). Nothing is frozen except work in progress:

| The new need arrives while | Do |
|---|---|
| any time | Add it to the introduction's *Later* list and as a `proposed` row in the index. Writing it down commits to nothing. |
| a feature is in phase 1 to 3 | If it does not change this feature's outcome, add it under *Out of scope* and go on. If it does, the human edits the brief or spec; that document goes back to `status: draft` and through its gate again. |
| its tickets are being built | Never change the current ticket. Finish or release it. The human decides whether it becomes a new ticket in this feature or a new feature. |
| the feature is `done` | A done feature is not reopened. The change becomes a new feature or new tickets. |

A new need that extends a planned feature is new evidence for its [evolution check](#evolution-check): ask about it, even if that feature's design is approved. An approved design whose evolution answer changes goes back to `draft` and through its gate again.

## Pitfalls

- **Writing the design during framing.** Tables, endpoints and crates in `brief.md` mean the problem is not yet understood.
- **Scenarios that test the framework** (JSON shape, status codes). Those belong to the adapter's tests, not to the spec.
- **One giant slice.** If a slice touches several aggregates or rules, split it.
- **Planning the next feature "while we're here".** Write it under *Out of scope* and stop.
- **Building for the future without being asked.** Generic parts come only from the [evolution check](#evolution-check), as data shape, and only when the human chooses it.
