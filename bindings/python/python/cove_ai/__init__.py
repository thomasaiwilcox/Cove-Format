from __future__ import annotations

import json
from pathlib import Path
from typing import Iterable, Iterator, Optional

from . import _native


def open(path: str | Path, cove_ai: str | Path | None = None, dataset_dir: str | Path | None = None) -> "TrainingArchive":
    return TrainingArchive(_native.open(str(path), None if cove_ai is None else str(cove_ai), None if dataset_dir is None else str(dataset_dir)))


class TrainingArchive:
    def __init__(self, native):
        self._native = native

    def verify(self, policy_report: bool = True):
        return self._native.verify(policy_report)

    def training_samples(self, split: Optional[str] = None, include_payloads: bool = False):
        return self._native.training_samples(split, include_payloads)

    def training_sample_count(self, split: Optional[str] = None) -> int:
        return self._native.training_sample_count(split)

    def iter_training_samples(self, split: Optional[str] = None, include_payloads: bool = False) -> Iterator[dict]:
        yield from self._native.iter_training_samples(split, include_payloads)

    def chunks(self, include_text: bool = False):
        return self._native.chunks(include_text)

    def tokens(self, include_payloads: bool = False):
        return self._native.tokens(include_payloads)

    def multimodal(self, include_payloads: bool = False):
        return self._native.multimodal(include_payloads)

    def to_hf_dataset(self, split: Optional[str] = None, streaming: bool = True, include_payloads: bool = True):
        try:
            from datasets import Dataset, IterableDataset
        except ImportError as exc:
            raise ImportError("Install cove-ai[hf] to use to_hf_dataset().") from exc

        if streaming:
            return IterableDataset.from_generator(
                lambda: self.iter_training_samples(split=split, include_payloads=include_payloads)
            )
        return Dataset.from_list(list(self.iter_training_samples(split=split, include_payloads=include_payloads)))

    def to_torch_iterable(self, split: Optional[str] = None, batch_size: int | None = None, include_payloads: bool = True):
        try:
            from torch.utils.data import IterableDataset
        except ImportError as exc:
            raise ImportError("Install cove-ai[torch] to use to_torch_iterable().") from exc

        archive = self

        class CoveAiIterableDataset(IterableDataset):
            def __iter__(self_inner):
                if batch_size is None:
                    yield from archive.iter_training_samples(split=split, include_payloads=include_payloads)
                    return
                batch = []
                for row in archive.iter_training_samples(split=split, include_payloads=include_payloads):
                    batch.append(row)
                    if len(batch) == batch_size:
                        yield batch
                        batch = []
                if batch:
                    yield batch

        return CoveAiIterableDataset()

    def export(self, format: str = "jsonl", out: str | Path | None = None, split: Optional[str] = None, include_payloads: bool = False):
        return self._native.export(format, None if out is None else str(out), split, include_payloads)

    def export_webdataset(self, out: str | Path, split: Optional[str] = None, include_payloads: bool = True):
        return self.export(format="webdataset", out=out, split=split, include_payloads=include_payloads)


__all__ = ["TrainingArchive", "open"]
