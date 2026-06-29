#!/usr/bin/env python3
from __future__ import annotations

import argparse

import cove_ai


def main() -> None:
    parser = argparse.ArgumentParser()
    parser.add_argument("archive")
    parser.add_argument("--split", default="train")
    parser.add_argument("--run-training", action="store_true")
    args = parser.parse_args()

    archive = cove_ai.open(args.archive)
    print(archive.verify(policy_report=True))
    dataset = archive.to_hf_dataset(split=args.split, streaming=True)

    if not args.run_training:
        for row in dataset:
            print(row)
            break
        return

    from trl import SFTTrainer

    raise SystemExit(
        "Pass a local model, tokenizer, and trainer config before constructing "
        "SFTTrainer. COVE-AI intentionally does not fetch network models."
    )


if __name__ == "__main__":
    main()
