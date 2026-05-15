"""Synthesize prompt text from a Mooncake trace row's hash_ids.

The Mooncake trace (Apache 2.0, kvcache-ai/Mooncake) does not contain
prompt text — privacy-redacted. Each row carries `input_length`,
`output_length`, and `hash_ids`: a list of integers identifying the
prefix-cache blocks the original request occupied. Rows that share a
prefix prefix share a `hash_ids` prefix.

To replay the trace's prefix-cache structure against any backend, we
synthesize a deterministic prompt where each `hash_id` maps to a fixed
16-token text chunk (vLLM's default block size). Concatenating chunks
in order reproduces the original block boundaries: rows in the same
prefix-cache cohort produce text with identical byte-prefix, so the
backend's RadixAttention sees the same KV-cache structure as the
original trace.

NVIDIA AIPerf and SGLang's Mooncake replay use the same approach.

Public surface:
    BLOCK_TOKENS                  Tokens per block (matches vLLM default).
    CHARS_PER_TOKEN               Coarse token estimator (~4).
    chunk_for_hash(h)             Deterministic 16-token text for one hash_id.
    synthesize_prompt(hash_ids, target_tokens)  Full prompt for one row.
"""

from __future__ import annotations

import hashlib

BLOCK_TOKENS = 16
CHARS_PER_TOKEN = 4
BLOCK_CHARS = BLOCK_TOKENS * CHARS_PER_TOKEN

# 64-char alphabet keyed by (hash bytes mod 64). Deterministic, plain ASCII so
# any tokenizer treats it consistently.
_ALPHABET = (
    "abcdefghijklmnopqrstuvwxyz"
    "ABCDEFGHIJKLMNOPQRSTUVWXYZ"
    "0123456789"
    "_-"
)
assert len(_ALPHABET) == 64

_PAD_TOKEN_TEXT = "x" * CHARS_PER_TOKEN  # one filler token


def _hash_bytes(h: int) -> bytes:
    """Stable byte digest for an integer hash_id."""
    raw = int(h).to_bytes(8, byteorder="little", signed=False)
    return hashlib.blake2b(raw, digest_size=BLOCK_CHARS).digest()


def chunk_for_hash(hash_id: int) -> str:
    """Deterministic ~16-token text chunk for a single hash_id.

    Same `hash_id` always returns the same chunk byte-for-byte. Different
    `hash_id` values almost always produce different chunks (collisions
    are vanishingly rare under blake2b).
    """
    digest = _hash_bytes(hash_id)
    return "".join(_ALPHABET[b % 64] for b in digest)


def synthesize_prompt(hash_ids: list[int], target_tokens: int) -> str:
    """Build a prompt that reproduces the trace row's prefix structure.

    The prompt is a concatenation of one chunk per `hash_id` in order,
    followed by deterministic padding to reach `target_tokens` (or
    truncation if over). The padding is identical bytes ("xxxx ") so it
    contributes a single shared filler block to every row's tail.
    """
    if target_tokens <= 0:
        return ""
    chunks = [chunk_for_hash(h) for h in hash_ids]
    body = "".join(chunks)
    target_chars = target_tokens * CHARS_PER_TOKEN
    if len(body) >= target_chars:
        return body[:target_chars]
    pad_needed = target_chars - len(body)
    return body + (_PAD_TOKEN_TEXT * (pad_needed // CHARS_PER_TOKEN))
