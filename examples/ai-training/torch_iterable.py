#!/usr/bin/env python3
from __future__ import annotations

import argparse

import cove_ai


def main() -> None:
    parser = argparse.ArgumentParser()
    parser.add_argument("archive")
    parser.add_argument("--split", default="train")
    parser.add_argument("--batch-size", type=int, default=2)
    args = parser.parse_args()

    archive = cove_ai.open(args.archive)
    print(archive.verify(policy_report=True))

    dataset = archive.to_torch_iterable(split=args.split, batch_size=args.batch_size)
    for batch in dataset:
        print(batch)
        break


if __name__ == "__main__":
    main()
