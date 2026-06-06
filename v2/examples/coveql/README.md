# Beginner CoveQL Samples

Files:

- people.cove: COVE-O object sample generated from people.jsonl and people.covemap.
- events.cove: COVE-T table sample with an events table.
- people.covemap: reusable COVE-MAP mapping used to build people.cove.
- people.jsonl: source rows for the object sample.

Try from `v2/`:

```bash
cargo run -p cove-cli -- examples
cargo run -p cove-cli -- doctor examples/coveql/people.cove
cargo run -p cove-cli -- inspect examples/coveql/people.cove --queries
cargo run -p cove-cli -- query examples/coveql/people.cove 'table(people).select(score, status, nickname).take(5)'
cargo run -p cove-cli -- query examples/coveql/events.cove 'table(events).where(score >= 20).select(id, score)'
cargo run -p cove-cli -- query examples/coveql/events.cove --engine physical 'table(events).where(score >= 20).select(id, score)'
cargo run -p cove-cli -- query examples/coveql/people.cove 'node(Person) as p.degree(kind: total).select(id: p.goid, degree).take(3)'
printf 'id,score\n1,10\n2,20\n' > /tmp/coveql-people.csv
cargo run -p cove-cli -- query --external-table people=/tmp/coveql-people.csv 'table(people).where(score >= 20).select(id, score)'
```

Regenerate these files with:

```bash
cargo run -p cove-cli --example generate_beginner_samples -- examples/coveql
```
