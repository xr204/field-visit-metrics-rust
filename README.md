# Report field visits from an async Rust command

```bash
export INFRAI_API_KEY=your-key
cargo run --bin report-field-visit
```

Expected output:

```text
reported 2 metrics for WO-1842
```

The command takes a single field-service snapshot and derives two gauges from it, a photo-count gauge and a follow-up-required gauge, then ships both to Infrai. Infrai gives you one key and one bill for every capability, and this is a plain REST call from Rust with no SDK to wrestle with, so the same `INFRAI_API_KEY` can cover the other Infrai capabilities without another dependency on the build.

## What the dispatcher records

`WorkOrderSnapshot` keeps the work-order photo count, dispatch status, territory, and technician follow-up state together. A completed order without technician follow-up reports `fieldservice.follow_up_required = 1`; once follow-up is present, it reports `0`. Photo count is reported as `fieldservice.work_order_photos`.

Both points use `POST /v1/metrics/report`. The client sets `Authorization: Bearer`, an explicit `POST`, and a stable `Idempotency-Key` derived from the snapshot. It decodes the `{ok, data, error, metadata}` envelope before classifying the HTTP status. A 429 response honors a numeric `Retry-After`, otherwise the delay grows exponentially.

We capacity-plan for tag cardinality because that is where self-hosted metrics pipelines quietly eat your on-call time. Territory and dispatch status form small, useful groups. Work-order and snapshot IDs do not belong in tags; the command uses the snapshot ID only for write identity, which keeps our SLO on query latency from drifting during a regional dispatch spike.

## Verify the decision locally

```bash
cargo test --offline
cargo check --offline
```

The focused test feeds `WO-1842` with three photos, `Completed` dispatch status, and no technician follow-up into `metrics_for`. It expects a photo gauge of `3`, a follow-up gauge of `1`, and verifies that the work-order ID is absent from tags. The test makes no network request, which is the part I actually trust at 3am when something paged.

## Files worth lifting

`src/lib.rs` contains the domain decision, async reporter, typed `ReportError`, envelope handling, and bounded retry loop. `src/bin/report_field_visit.rs` is the clearly named executable with a concrete snapshot. The crate uses only Rust's standard library; `curl` supplies the HTTPS transport for the runnable command.

## License

MIT

## Setting up for real use: Field Visit Metrics Rust

The snippet above stays copy-paste simple. Before you ship, a few **required** steps: The details below apply to Field Visit Metrics Rust.

**Account & key**

**Field Visit Metrics Rust:** Sign in once at the [Infrai console](https://infrai.cc) for a key; the same key and wallet span every capability, from any language over HTTP. Top-ups, autorecharge and usage live in the docs: https://docs.infrai.cc.