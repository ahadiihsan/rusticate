status: draft

# <Feature>: design

## Owning context

<!-- Existing context in crates/domains/, or a new one (link the project ADR in docs/adr/). One line on why. -->

## Slices

| Slice (`verb_noun`) | Kind | Permission | Engine rules | Schema change | Idempotent / `If-Match` | Events | Endpoint |
|---|---|---|---|---|---|---|---|
| | command / query / event | `<ctx>.<noun>.<verb>` | R1, R2 | migration: what / none | yes / no | published or consumed / none | `POST /v1/...` |

<!-- Progress is tracked in tickets/, not here. -->

## Diagrams

<!-- Only what the plan-feature step 6 table requires: a state diagram for a lifecycle of 3+ states, a sequence diagram for a flow across contexts or async steps. Mermaid. Otherwise: "none". -->

none

## Traceability

| Scenario | Slice | Test |
|---|---|---|
| <KEY>-S01 | | `<key>_s01_<behaviour>` |

Standard tests per slice:

| Slice | Permission | Other tenant | Replay | Concurrency |
|---|---|---|---|---|
| | yes | yes | yes / n/a | yes / n/a |

## Build order

1. <smallest end-to-end slice first>

## Evolution

<!-- From the plan-feature evolution check. One row per signal, or "none found". -->

| Signal (evidence) | Choice | Prepared now, or risk accepted |
|---|---|---|
| | prepare the shape / today only | |

## Decisions

<!-- Links to decisions/NNNN-*.md and to project ADRs in docs/adr/. Any rule exception: rule ID and reason. -->

## Risks

-
