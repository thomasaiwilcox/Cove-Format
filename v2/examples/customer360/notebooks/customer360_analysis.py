#!/usr/bin/env python3
import argparse
import json
from collections import Counter
from pathlib import Path

def load_jsonl(path):
    rows = []
    with path.open("r", encoding="utf-8") as handle:
        for line in handle:
            line = line.strip()
            if line:
                rows.append(json.loads(line))
    return rows

def main():
    parser = argparse.ArgumentParser(description="Notebook-style Customer 360 analysis")
    parser.add_argument("--input-dir", default=".", help="Generated Customer 360 directory")
    args = parser.parse_args()
    root = Path(args.input_dir)
    manifest = json.loads((root / "customer360-manifest.json").read_text(encoding="utf-8"))
    support = load_jsonl(root / "support.jsonl")
    events = load_jsonl(root / "events.jsonl")

    print("Customer 360 profile:", manifest["profile"])
    print("Source rows:", manifest["row_counts"])
    print("Support status distribution:", dict(Counter(row["status"] for row in support)))
    print("Event kind distribution:", dict(Counter(row["event_kind"] for row in events)))

    try:
        import pandas as pd
        support_df = pd.DataFrame(support)
        events_df = pd.DataFrame(events)
        print("\nPandas status by active flag:")
        print(support_df.groupby(["status", "active"]).size().reset_index(name="rows").head(10))
        print("\nPandas event score summary:")
        print(events_df.groupby("event_kind")["score"].agg(["count", "mean"]).reset_index())
    except Exception as exc:
        print("\nPandas section skipped:", exc)

    try:
        import polars as pl
        support_pl = pl.DataFrame(support)
        print("\nPolars average score by status:")
        print(support_pl.group_by("status").agg(pl.col("score").mean().alias("avg_score")))
    except Exception as exc:
        print("\nPolars section skipped:", exc)

    print("\nRun these CLI queries for canonical rows and provenance:")
    for item in manifest["recommended_queries"]:
        print("-", item["command"])

if __name__ == "__main__":
    main()
