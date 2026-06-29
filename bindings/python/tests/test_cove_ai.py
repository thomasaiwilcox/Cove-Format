import json
import subprocess
from pathlib import Path

import cove_ai


def test_open_verify_and_iterate_fixture(tmp_path: Path):
    source = tmp_path / "samples.jsonl"
    source.write_text(
        json.dumps(
            {
                "sample_id": "py-1",
                "instruction": "Explain COVE-AI.",
                "input": "archive",
                "output": "policy-aware training data",
            }
        )
        + "\n"
    )
    archive_path = tmp_path / "training.coveai"
    subprocess.run(
        [
            "cargo",
            "run",
            "-p",
            "cove-cli",
            "--",
            "ai",
            "import",
            "jsonl",
            str(source),
            "--out",
            str(archive_path),
            "--schema",
            "instruction",
        ],
        check=True,
        cwd=Path(__file__).resolve().parents[3],
    )

    archive = cove_ai.open(archive_path)
    report = archive.verify()
    assert report["training_sample_count"] == 1
    rows = archive.training_samples(include_payloads=True)
    assert rows[0]["input"]["payload_access"] == "allowed"
