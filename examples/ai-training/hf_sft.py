#!/usr/bin/env python3
from __future__ import annotations

import argparse

import cove_ai


def main() -> None:
    parser = argparse.ArgumentParser()
    parser.add_argument("archive")
    parser.add_argument("--split", default="train")
    args = parser.parse_args()

    archive = cove_ai.open(args.archive)
    report = archive.verify(policy_report=True)
    print(report)

    dataset = archive.to_hf_dataset(split=args.split, streaming=True)
    for row in dataset:
        print(row)
        break


if __name__ == "__main__":
    main()
