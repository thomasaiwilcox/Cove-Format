# COVE Reference Showcase

This directory is a compact end-to-end showcase for the unified `cove` CLI.

Files:

- `crm_people.jsonl`: CRM source rows with names, regions, and tiers.
- `support_people.jsonl`: support source rows with activity, scores, and statuses.
- `customer_identity.covemap`: COVE-MAP identity and projection metadata for both sources.
- `customers_360.jsonl`: reconciled readback rows derived from the two source shapes.
- `customer_readback.covemap`: COVE-MAP metadata used to generate the queryable COVE-O file.
- `customers.cove`: generated COVE-O object file with projection and evidence metadata.
- `events.cove`: companion COVE-T table with event scores by source person id.

Try from `v2/`:

```bash
cargo run -p cove-cli -- doctor examples/showcase/customers.cove
cargo run -p cove-cli -- inspect --queries --performance examples/showcase/customers.cove
cargo run -p cove-cli -- map project --format json examples/showcase/customer_identity.covemap examples/showcase/crm_people.jsonl examples/showcase/support_people.jsonl
cargo run -p cove-cli -- query examples/showcase/customers.cove 'table(customers).select(full_name, region, tier, score, status).orderBy(score, desc)'
cargo run -p cove-cli -- query examples/showcase/customers.cove 'evidence().select(source_id, source_row_identity, rule_id).take(10)'
cargo run -p cove-cli -- query examples/showcase/events.cove 'table(events).where(score >= 25).select(event_id, person_id, score)'
cargo run -p cove-cli -- optimize examples/showcase/events.cove
cargo run -p cove-cli -- query --engine compare --perf-report examples/showcase/events.cove 'table(events).where(score >= 25).select(event_id, person_id, score)'
cargo run -p cove-cli -- query --format jsonl examples/showcase/customers.cove 'table(customers).select(full_name, score, status)'
cargo run -p cove-cli -- query --format csv examples/showcase/events.cove 'table(events).select(event_id, person_id, score)'
```

Regenerate these files with:

```bash
cargo run -p cove-cli --example generate_beginner_samples -- --showcase examples/showcase
```
