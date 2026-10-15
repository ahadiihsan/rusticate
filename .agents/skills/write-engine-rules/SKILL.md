---
name: write-engine-rules
description: Implement business rules, calculations, money arithmetic, validation of invariants or entity lifecycles as pure functions in a <context>-engine crate, with example and property-based tests. Use whenever a feature has rules more complex than storing and returning data.
---

# Write engine rules

Engines are the pure core of a context (`cargo rusticate new domain` creates one next to every context). They make the hardest logic the cheapest to test: there is no database, no async, and the suite runs in milliseconds.

## Constraints (enforced)

| Rule | Meaning | Enforced by |
|---|---|---|
| ENG-001 | dependencies only from the allowlist (`serde`, `thiserror`; dev: `proptest`, `rust_decimal`); no `async`/`.await`; no I/O | `cargo rusticate arch`, engine `clippy.toml` |
| ENG-002 | no clock, no id generation, no randomness, no `HashMap`/`HashSet` iteration: pass `now`, ids and seeds **in** | engine `clippy.toml` |
| ENG-003 | money is `Money` (i64 minor units + `Currency`); rates are `Bps`; no `f32`/`f64` anywhere | `arch`, clippy, types |
| ENG-004 | `#![deny(clippy::arithmetic_side_effects, clippy::as_conversions)]`: use `checked_*`, `try_from`, `saturating_*` deliberately | clippy |

## Shape

```rust
/// What the rule needs, as validated domain types.
pub struct ShippingInput { pub weight_grams: Grams, pub zone: Zone, pub rate_card: RateCard }

/// What the rule decides.
pub struct ShippingQuote { pub price: Money, pub service_level: ServiceLevel }

/// Pure: same input, same output; every failure is a typed variant.
pub fn quote_shipping(input: &ShippingInput) -> Result<ShippingQuote, EngineError> { ... }
```

- One `EngineError` enum per engine, with **one variant per rule**, carrying the data needed for a good message (`TooManyLines { max }`). The context maps each variant to a stable `error_code` in `error.rs`.
- **Lifecycles** are enums with one exhaustive `apply(self, action) -> Result<Self, EngineError>` and **no `_ =>` arm**, so a new state or action will not compile until every transition is decided.
- **Rounding** happens once, at a documented point, half-to-even (`Money::apply_rate`). Rounding per line lets error accumulate.
- Keep amounts within ±(2⁵³−1) (`MAX_MINOR`): JSON clients parse numbers as doubles.

## Tests (ENG-005)

Keep tests in `#[cfg(test)] mod tests` next to the code. Write both kinds:

**Examples** pin known answers, including the awkward ones: boundaries, zero, maximum, rounding ties (`2.5 → 2`, `3.5 → 4`), and every error variant.

**Properties** must not restate the implementation. A property that recomputes the result the same way the code does proves nothing (that was v1's mistake). Instead, use one of:

| Kind | Example |
|---|---|
| Independent oracle | engine rounding == `rust_decimal` `MidpointNearestEven` on the exact value |
| Conservation | `total == subtotal + tax`; `subtotal ==` the exact `i128` sum of the lines |
| Invariance | reordering lines does not change totals |
| Bounds | `0 ≤ tax ≤ subtotal` for rates ≤ 100 % |
| Totality | for all inputs: `Ok(_)` or a typed `Err`, **never a panic** (overflow included) |
| Round trip | `parse(x.as_str()) == x` for every state |

```rust
proptest! {
    #[test]
    fn tax_matches_decimal_half_even_oracle(subtotal in 0..=MAX_MINOR, bps in 0..=Bps::MAX_VALUE) {
        let engine = idr(subtotal).apply_rate(Bps::new(bps).unwrap()).unwrap().minor();
        let exact = Decimal::from(subtotal) * Decimal::from(bps) / Decimal::from(10_000);
        prop_assert_eq!(engine, exact.round_dp_with_strategy(0, RoundingStrategy::MidpointNearestEven).to_i64().unwrap());
    }
}
```

Test-only arithmetic in oracles may need `#[expect(clippy::arithmetic_side_effects, reason = "...")]` on the test module, never on production code.

Run with `cargo rusticate test -p <ctx>-engine`.

## Using the engine from a workflow

The workflow loads facts (settings, current state) inside the transaction, calls the engine, and persists the result. When an engine input comes from the database, convert it with domain constructors and map failures to `<Ctx>Error::Corrupt`.
<!-- rusticate:module(example):start -->

## Example

[`billing-engine`](../../../crates/domains/billing-engine) applies every rule above: integer money with a currency, a lifecycle ([`InvoiceStatus::apply`](../../../crates/domains/billing-engine/src/invoice.rs)), and property tests against an independent decimal oracle.
<!-- rusticate:module(example):end -->
