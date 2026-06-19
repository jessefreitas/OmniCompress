"""Reversibility proof against the real library (skipped if the wheel is absent)."""
import json

import pytest

omnicompress = pytest.importorskip("omnicompress")


def _big_array(n: int = 25) -> str:
    return json.dumps(
        [{"svc": f"svc-{i:02d}", "region": "us-east-1", "blob": "v" * 12} for i in range(n)]
    )


def test_ccr_roundtrip_recovers_exact_original():
    # Big array (>=20 rows) buried early so it gets compressed (outside protect_recent).
    ctx = [{"role": "user", "content": "dump the registry"},
           {"role": "tool", "content": _big_array(25)}]
    ctx += [{"role": "user", "content": f"step {i}"} for i in range(6)]

    sess = omnicompress.OmniCompressSession()
    comp = sess.compress(ctx)

    assert comp["tokens_after"] < comp["tokens_before"], "compression should shrink it"
    assert comp["ccr_refs"], "compression should have stored the original in the CCR"

    # The compressed view dropped detail; the CCR returns it byte-identical.
    h = comp["ccr_refs"][0]["hash"]
    assert sess.retrieve(h) == ctx[1]["content"]


def test_marker_carries_the_hash_for_retrieval():
    ctx = [{"role": "user", "content": "dump"}, {"role": "tool", "content": _big_array(25)}]
    ctx += [{"role": "user", "content": f"step {i}"} for i in range(6)]
    sess = omnicompress.OmniCompressSession()
    comp = sess.compress(ctx)
    h = comp["ccr_refs"][0]["hash"]
    # the model must be able to find the hash in what it sees
    assert h in comp["messages"][1]["content"]
