"""
TDD tests for the omnicompress Python binding.
These must FAIL before `maturin develop` and PASS after.
"""
import json
import omnicompress


def _big_json(n: int = 60) -> str:
    return json.dumps([{"id": i, "v": str(i)} for i in range(n)])


def test_lossless_default_shrinks_without_ccr():
    """Default mode is LOSSLESS: a large JSON tool result becomes a compact
    columnar table that keeps every row in the visible content, so NOTHING is
    stored in the CCR (no retrieve loop required)."""
    msgs = [{"role": "user", "content": _big_json(), "tool_name": "search"}]
    msgs += [{"role": "assistant", "content": f"ok {i}"} for i in range(6)]
    res = omnicompress.compress(msgs)  # lossless=True by default
    assert res["tokens_after"] < res["tokens_before"], (
        f"expected compression: {res['tokens_before']} -> {res['tokens_after']}"
    )
    assert len(res["ccr_refs"]) == 0, (
        f"lossless mode must not write the CCR, got {len(res['ccr_refs'])} refs"
    )
    # The columnar table is visible in the content (self-contained).
    assert "json_table" in res["messages"][0]["content"]


def test_aggressive_mode_records_ccr_ref():
    """With lossless=False the array is sampled/elided and the original is stored
    in the CCR — exactly 1 ref recorded."""
    msgs = [{"role": "user", "content": _big_json(), "tool_name": "search"}]
    msgs += [{"role": "assistant", "content": f"ok {i}"} for i in range(6)]
    res = omnicompress.compress(msgs, lossless=False)
    assert res["tokens_after"] < res["tokens_before"]
    assert len(res["ccr_refs"]) == 1, (
        f"expected 1 CCR ref in aggressive mode, got {len(res['ccr_refs'])}"
    )


def test_cache_stable_compresses_even_the_recent_window():
    """cache_stable ignores the recent-N protection, so even a lone recent block
    is compressed — keeping the prompt prefix byte-stable across turns."""
    res = omnicompress.compress(
        [{"role": "user", "content": _big_json()}], cache_stable=True
    )
    assert res["tokens_after"] < res["tokens_before"], (
        "cache_stable should compress even the most-recent message"
    )


def test_fail_open_short_content_untouched():
    """A short message must pass through unchanged (min_chars_to_compress = 600)."""
    res = omnicompress.compress([{"role": "user", "content": "curto"}])
    assert res["messages"][0]["content"] == "curto", (
        f"short content must be untouched, got: {res['messages'][0]['content']}"
    )
