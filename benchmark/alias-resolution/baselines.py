"""Frozen free baselines on historical text; no stored vectors are read."""

import hashlib
import json
import re
import unicodedata
from pathlib import Path


def normalized_label(label):
    text = unicodedata.normalize("NFKC", label).casefold()
    return " ".join("".join(c if c.isalnum() else " " for c in text).split())


def label_score(left, right):
    """Registered normalized Levenshtein score, unavailable for empty labels."""
    left, right = normalized_label(left), normalized_label(right)
    if not left or not right:
        return None
    previous = list(range(len(right) + 1))
    for i, a in enumerate(left, 1):
        current = [i]
        for j, b in enumerate(right, 1):
            current.append(
                min(current[-1] + 1, previous[j] + 1, previous[j - 1] + (a != b))
            )
        previous = current
    return 1.0 - previous[-1] / max(len(left), len(right))


def label(record):
    # Multiple labels are sorted by preparation, never selected using gold.
    return "\n".join(sorted(record["labels"]))


def text(record):
    return "\n".join(
        (label(record), record["description"], " ".join(sorted(record["types"])))
    )


def commit_identity(record):
    """Use explicit hash/repository fields or identifier-shaped labels only."""
    hashes, repos = set(), set()
    for edge in record["edges"]:
        predicate = edge["predicate"].rsplit("/", 1)[-1].rsplit("#", 1)[-1]
        value = edge["value"]
        if (
            predicate in ("hash", "sha")
            and "text" in value
            and re.fullmatch(r"[0-9a-f]{7,40}", value["text"])
        ):
            hashes.add(value["text"])
        if predicate in ("repo", "in_repo", "committed_to"):
            repo = value.get("text") or value.get("iri", "").rsplit("/", 1)[-1]
            if repo:
                repos.add(repo)
    for name in record["labels"]:
        for pattern in (
            r"^(?:code/)?commit/([^/]+)/([0-9a-f]{7,40})$",
            r"^([^@ ]+)@([0-9a-f]{7,40})(?::.*)?$",
        ):
            match = re.fullmatch(pattern, name)
            if match:
                repos.add(match[1])
                hashes.add(match[2])
        match = re.fullmatch(r"(?:commit[-_])?([0-9a-f]{7,40})", name)
        if match:
            hashes.add(match[1])
    if len(repos) != 1 or not hashes:
        return None
    longest = max(hashes, key=len)
    if not all(longest.startswith(h) for h in hashes):
        return None
    return next(iter(repos)), longest


def id_control(left, right, historical_pool):
    a, b = commit_identity(left), commit_identity(right)
    if a is None or b is None or a[0] != b[0]:
        return None
    if not (a[1].startswith(b[1]) or b[1].startswith(a[1])):
        return 0.0
    prefix = min((a[1], b[1]), key=len)
    matches = {
        identity[1]
        for record in historical_pool
        if (identity := commit_identity(record)) is not None
        and identity[0] == a[0]
        and identity[1].startswith(prefix)
    }
    # Collapse known shortened spellings; two incompatible extensions abstain.
    maximal = {
        h
        for h in matches
        if not any(other != h and other.startswith(h) for other in matches)
    }
    if len(maximal) != 1 or len(next(iter(maximal))) != 40:
        return None
    return 1.0


class Encoder:
    def __init__(self, model, tokenizer, max_length=256):
        import numpy as np
        import onnxruntime as ort
        from tokenizers import Tokenizer

        self.np = np
        options = ort.SessionOptions()
        options.intra_op_num_threads = 1
        options.enable_cpu_mem_arena = False
        options.enable_mem_pattern = False
        self.session = ort.InferenceSession(
            str(model), sess_options=options, providers=["CPUExecutionProvider"]
        )
        self.tokenizer = Tokenizer.from_file(str(tokenizer))
        self.tokenizer.enable_truncation(max_length=max_length)
        padding = self.tokenizer.padding
        if padding:
            padding.pop("length", None)
            padding.pop("pad_to_multiple_of", None)
            self.tokenizer.enable_padding(**padding)
        self.manifest = {
            "model_sha256": hashlib.sha256(Path(model).read_bytes()).hexdigest(),
            "tokenizer_sha256": hashlib.sha256(
                Path(tokenizer).read_bytes()
            ).hexdigest(),
            "max_tokens": max_length,
            "special_tokens": True,
            "pooling": "attention-mask mean",
            "normalization": "L2",
            "padding": "batch longest",
            "onnxruntime": ort.__version__,
            "numpy": np.__version__,
            "text": "labels, description, sorted type IRIs",
        }

    def encode(self, texts, batch_size=32):
        np = self.np
        vectors = []
        for start in range(0, len(texts), batch_size):
            encoded = self.tokenizer.encode_batch(texts[start : start + batch_size])
            inputs = {
                "input_ids": np.array([e.ids for e in encoded], dtype=np.int64),
                "attention_mask": np.array(
                    [e.attention_mask for e in encoded], dtype=np.int64
                ),
                "token_type_ids": np.array(
                    [e.type_ids for e in encoded], dtype=np.int64
                ),
            }
            output = self.session.run(None, inputs)[0]
            if output.ndim != 3 or output.shape[2] != 384:
                raise ValueError("expected source MiniLM token output of dimension 384")
            mask = inputs["attention_mask"].astype(np.float32)[..., None]
            pooled = (output * mask).sum(axis=1) / mask.sum(axis=1)
            norm = np.linalg.norm(pooled, axis=1, keepdims=True)
            if not np.isfinite(pooled).all() or (norm == 0).any():
                raise ValueError("invalid embedding")
            vectors.extend((pooled / norm).tolist())
        return vectors


def input_hash(state):
    """Content addressing uses the evidence, never a gold label or pair ID."""
    encoded = json.dumps(
        state, ensure_ascii=False, sort_keys=True, separators=(",", ":")
    ).encode()
    return hashlib.sha256(encoded).hexdigest()
