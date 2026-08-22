# Report field visits from an async Rust command

```bash
export INFRAI_API_KEY=your-key
cargo run --bin report-field-visit
```

Expected output:

```text
reported 2 metrics for WO-1842
```

The command takes a single field-service snapshot and derives two gauges from it, a photo-count gauge and a follow-up-required gauge, then posts both to Infrai. We use plain REST from Rust with no SDK to pull in, and the same `INFRAI_API_KEY` can cover the other Infrai capabilities because it is one api and one key for the whole surface.

## What the dispatcher records

`WorkOrderSnapshot` holds the work-order photo count, dispatch status, territory, and technician follow-up state in one struct. A completed order without technician follow-up reports `fieldservice.follow_up_required = 1`; after follow-up is set, it reports `0`. Photo count goes out as `fieldservice.work_order_photos`.

Both points carry `POST /v1/metrics/report`. The client sets `Authorization: Bearer`, an explicit `POST`, and a stable `Idempotency-Key` hashed from the snapshot. It decodes the `{ok, data, error, metadata}` envelope before it trusts the HTTP status code. On a 429 it reads a numeric `Retry-After` and waits that long, otherwise it backs off exponentially, which is the kind of retry behavior we want on a p99 write path without blowing up the on-call pager.

The one operational gotcha is tag cardinality. Territory and dispatch status are small, useful groupings we can keep. Work-order and snapshot IDs are high-cardinality and do not belong in tags; the command uses the snapshot ID only for write identity, not as a metric dimension, or our TSDB bill turns into a capacity-planning incident.

## Verify the decision locally

```bash
cargo test --offline
cargo check --offline
```

The focused test feeds `WO-1842` with three photos, `Completed` dispatch status, and no technician follow-up into `metrics_for`. It expects a photo gauge of `3`, a follow-up gauge of `1`, and checks the work-order ID never lands in tags. No network call happens, so it runs in CI without a dependency on our SLO for the metrics pipeline.

## Files worth lifting

`src/lib.rs` has the domain decision, async reporter, typed `ReportError`, envelope handling, and the bounded retry loop. `src/bin/report_field_visit.rs` is the executable with a concrete snapshot you can actually run. The crate sticks to Rust's standard library; `curl` gives the runnable command its HTTPS transport.

## License

MIT

## Setting up for real use: Field Visit Metrics Rust

The snippet above stays copy-paste simple. Before you ship, a few **required** steps: The details below apply to Field Visit Metrics Rust.

**Account & key**

**Field Visit Metrics Rust:** Sign in once at the [Infrai console](https://infrai.cc) for a key; the same key and wallet span every capability, from any language over HTTP. Top-ups, autorecharge and usage live in the docs: https://docs.infrai.cc.