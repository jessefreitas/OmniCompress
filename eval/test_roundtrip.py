"""Reversibility proof against the real library (skipped if the wheel is absent)."""
import json

import pytest

omnicompress = pytest.importorskip("omnicompress")


def _big_array(n: int = 25) -> str:
    return json.dumps(
        [{"svc": f"svc-{i:02d}", "region": "us-east-1", "blob": "v" * 12} for i in range(n)]
    )


def test_ccr_roundtrip_recovers_exact_original():
    # Aggressive mode (lossless=False) stores the original in the CCR; the big
    # array (>=20 rows) is buried early so it gets compressed (outside protect_recent).
    ctx = [{"role": "user", "content": "dump the registry"},
           {"role": "tool", "content": _big_array(25)}]
    ctx += [{"role": "user", "content": f"step {i}"} for i in range(6)]

    sess = omnicompress.OmniCompressSession()
    comp = sess.compress(ctx, lossless=False)

    assert comp["tokens_after"] < comp["tokens_before"], "compression should shrink it"
    assert comp["ccr_refs"], "aggressive mode should have stored the original in the CCR"

    # The compressed view dropped detail; the CCR returns it byte-identical.
    h = comp["ccr_refs"][0]["hash"]
    assert sess.retrieve(h) == ctx[1]["content"]


def test_marker_carries_the_hash_for_retrieval():
    ctx = [{"role": "user", "content": "dump"}, {"role": "tool", "content": _big_array(25)}]
    ctx += [{"role": "user", "content": f"step {i}"} for i in range(6)]
    sess = omnicompress.OmniCompressSession()
    comp = sess.compress(ctx, lossless=False)
    h = comp["ccr_refs"][0]["hash"]
    # the model must be able to find the hash in what it sees
    assert h in comp["messages"][1]["content"]


def test_lossless_columnar_keeps_all_data_without_ccr():
    # Default lossless mode: the array becomes a self-contained columnar table.
    # No CCR write, yet every row is preserved in the visible content.
    ctx = [{"role": "user", "content": "dump the registry"},
           {"role": "tool", "content": _big_array(25)}]
    ctx += [{"role": "user", "content": f"step {i}"} for i in range(6)]

    sess = omnicompress.OmniCompressSession()
    comp = sess.compress(ctx)  # lossless=True by default

    assert comp["tokens_after"] < comp["tokens_before"], "columnar form should shrink it"
    assert not comp["ccr_refs"], "lossless mode must not touch the CCR"

    table = json.loads(comp["messages"][1]["content"])
    assert table["_omnicompress"] == "json_table"
    assert table["count"] == 25, "every row must be accounted for"
