"""
Task 13 — TDD tests for the omnicompress Python binding.
These must FAIL before `maturin develop` and PASS after.
"""
import json
import omnicompress


def test_compress_returns_metrics_and_shrinks_old_tool_json():
    """A large JSON tool result (60 items) at position 0 with 6 trailing prose
    blocks must be compressed; exactly 1 CCR ref must be recorded."""
    big = json.dumps([{"id": i, "v": str(i)} for i in range(60)])
    msgs = [{"role": "user", "content": big, "tool_name": "search"}]
    msgs += [{"role": "assistant", "content": f"ok {i}"} for i in range(6)]
    res = omnicompress.compress(msgs)
    assert res["tokens_after"] < res["tokens_before"], (
        f"expected compression: {res['tokens_before']} -> {res['tokens_after']}"
    )
    assert len(res["ccr_refs"]) == 1, (
        f"expected 1 CCR ref, got {len(res['ccr_refs'])}"
    )


def test_fail_open_short_content_untouched():
    """A short message must pass through unchanged (min_chars_to_compress = 600)."""
    res = omnicompress.compress([{"role": "user", "content": "curto"}])
    assert res["messages"][0]["content"] == "curto", (
        f"short content must be untouched, got: {res['messages'][0]['content']}"
    )
