status: draft
scenario key: <KEY>

# <Feature>: spec

## Glossary

| Term | Meaning |
|---|---|
| | |

## Rules and scenarios

### R1. <rule in plain language> (calculation | invariant | policy)

```gherkin
# <KEY>-S01
Given <state>
When <action>
Then <outcome>
```

```gherkin
# <KEY>-S02  (rejection)
Given <state>
When <action that breaks R1>
Then it is rejected because <reason>
```

<!-- Not listed here (every slice gets them from the design): missing permission, another tenant, idempotent replay, concurrency. -->

## Open questions

| # | Question | Owner | Answer |
|---|---|---|---|
| Q1 | | | |
