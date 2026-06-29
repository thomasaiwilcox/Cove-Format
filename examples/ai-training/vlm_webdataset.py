#!/usr/bin/env python3
from __future__ import annotations

import argparse
from pathlib import Path

import cove_ai


def main() -> None:
    parser = argparse.ArgumentParser()
    parser.add_argument("archive")
    parser.add_argument("--out", default="target/cove-ai-training/vlm-training.tar")
    parser.add_argument("--split", default="train")
    args = parser.parse_args()

    archive = cove_ai.open(args.archive)
    print(archive.verify(policy_report=True))
    out = Path(args.out)
    out.parent.mkdir(parents=True, exist_ok=True)
    archive.export_webdataset(out, split=args.split, include_payloads=True)
    print(out)


if __name__ == "__main__":
    main()
