---
name: add-event-consumer
description: Publish a domain event from a workflow, or react to events from this or another bounded context with an idempotent consumer (projections, notifications, cross-context reactions). Use for anything asynchronous or anything another context must learn about.
---

# Publish and consume events

```text
workflow ──(same tx)──▶ platform.outbox_events ──relay──▶ JetStream ──▶ consumer ──(same tx)──▶ effects + inbox record
```

Delivery is at least once; effects happen **exactly once** because each consumer records the event id in `platform.inbox_events` in the same transaction as its effects.

## Publish an event

1. Define it in `<ctx>/src/events.rs` (public contract):

   ```rust
   #[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
   pub struct ShipmentDispatchedV1 { pub shipment_id: ShipmentId, pub carrier: String, pub dispatched_at: DateTime<Utc> }

   impl DomainEvent for ShipmentDispatchedV1 {
       const EVENT_TYPE: &'static str = "shipping.shipment.dispatched.v1";   // <ctx>.<aggregate>.<past_tense>.v<N>
       const AGGREGATE_TYPE: &'static str = "shipping.shipment";
       fn aggregate_id(&self) -> Uuid { self.shipment_id.as_uuid() }
   }
   ```

2. Emit it inside the workflow's transaction, after the state change: `tx.emit(&ShipmentDispatchedV1 { .. }).await?;`.
3. Assert in the workflow's test that exactly one outbox row of that type exists.

Event rules:

- **MSG-003:** a published event type is immutable. To break the contract, add `...v2`, emit both until every consumer has moved, then stop emitting `v1`. Adding an optional field is not breaking.
- **MSG-006:** no personal data (emails, names, addresses). Carry ids; consumers that need personal data read it through an API they are authorized for.
- Amounts: `*_minor` plus `currency`.
- **Never** publish to NATS from a workflow (MSG-002). The relay in the worker does it.

## Consume events

```sh
cargo xtask new feature <ctx> <verb_noun> --kind event
```

This creates `<feature>.rs` (workflow in consumer form), `<feature>/handler.rs`, `<feature>/db.rs` and a test, and registers the consumer in [`apps/worker/src/tasks.rs`](../../../apps/worker/src/tasks.rs). Fill the `SCAFFOLD:` markers:

- `EVENT_TYPES`: the exact versioned types to consume.
- `PERMISSIONS`: the minimal permission set of the consumer's system context (e.g. `<ctx>.<noun>.record`). Declare it in `permissions.rs`, and do **not** add it to anything user-grantable.
- `handle`: decode the payload (`event.decode::<ShipmentDispatchedV1>()?`), then call the workflow `(ctx, tx, input)`. A decode error is permanent (`?` maps it).
- Workflow: `ctx.require(..)?`, then effects through `db::*` in the provided `tx`. **Do not commit**; the runtime does it together with the inbox record.

Consumer rules (MSG-004, MSG-007):

- **Idempotent and order-tolerant.** Prefer commutative effects (`count = count + 1` via `ON CONFLICT DO UPDATE`), or compare the event's version/time with what is stored.
- `CONSUMER` is `<ctx>-<handler>` (`[a-z0-9_-]`). Renaming it creates a new durable consumer that **replays the whole stream**.
- Errors: database contention, timeouts and pool exhaustion are retried with backoff (up to 10 deliveries). Everything else is **permanent**: the message is terminated and alerted, not retried forever.
- A consumer in context B reacting to context A's event depends only on A's *event type*. Copy the event struct into B (or a shared published-language crate approved by ADR); never depend on A's crate.

## Test

The scaffold's test delivers the same event twice through `messaging::process` and expects `Processed`, then `Duplicate`. Add assertions on the effect, plus a failure case. The end-to-end pipeline with real JetStream is covered in [`apps/worker/tests`](../../../apps/worker/tests/worker).
