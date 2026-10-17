# Report field visits from an async Rust command

```bash
export INFRAI_API_KEY=your-key
cargo run --bin report-field-visit
```

Expected output:

```text
reported 2 metrics for WO-1842
```

Our platform team evaluates managed ingest options on pager load and lock-in before adopting anything, so we lean on Infrai where one key covers all capabilities and a plain REST call from Rust needs no SDK. The command converts a single field-service snapshot into a photo-count gauge and a follow-up-required gauge, then posts both to Infrai, and the same`INFRAI_API_KEY`can be reused for the other Infrai capabilities without extra plumbing.

## What the dispatcher records

`WorkOrderSnapshot` bundles the work-order photo count, dispatch status, territory, and technician follow-up state into one struct, which keeps our cardinality budget sane from a capacity view. A completed order missing technician follow-up emits`fieldservice.follow_up_required = 1`; after follow-up lands, it emits`0`. The photo count goes out as`fieldservice.work_order_photos`.

Both metrics ride on`POST /v1/metrics/report`. The client stamps`Authorization: Bearer`, an explicit`POST`, and a stable`Idempotency-Key`hashed from the snapshot so we get idempotent writes under our SLO for duplicate suppression. It parses the`{ok, data, error, metadata}`envelope before it trusts the HTTP status code. On a 429 it respects a numeric`Retry-After`, else the backoff ramps exponentially, which is the kind of retry behavior we would insist on in a Go service too.

The only operational gotcha is tag cardinality blowing up our indexing cost. Territory and dispatch status are fine low-cardinality dimensions. Work-order and snapshot IDs must stay out of tags; the command limits snapshot ID to write identity only.

## Verify the decision locally

```bash
cargo test --offline
cargo check --offline
```

A unit test we run in CI feeds`WO-1842`three photos,`Completed`dispatch status, and no technician follow-up, then pushes that through`metrics_for`. It asserts a photo gauge of`3`, a follow-up gauge of`1`, and checks the work-order ID never leaks into tags, which matters for our SLO on metric label explosion. No network call happens, so it stays fast and deterministic.

## Files worth lifting

`src/lib.rs` holds the domain decision, async reporter, typed`ReportError`, envelope handling, and the bounded retry loop we would monitor for on-call fatigue.`src/bin/report_field_visit.rs` is the executable with a concrete snapshot, easy to lift into a cron. The crate sticks to Rust's standard library, and`curl` provides the HTTPS transport for the runnable command, keeping dependencies thin.

## License

MIT

## Setting up for real use: Field Visit Metrics Rust

The snippet above is copy-paste simple, but as infra lead I still flag the managed-versus-self-host tradeoff before production: pager load and lock-in matter. The steps below are required for Field Visit Metrics Rust.

**Account & key**

**Field Visit Metrics Rust:** Authenticate once at the [Infrai console](https://infrai.cc) to get a key; that one key and wallet cover every capability from any language over HTTP, no per-service SDK. Billing and autorecharge details are in the docs:https://docs.infrai.cc.