import json

import omnicompress


def test_session_compress_retrieve_roundtrip():
    sess = omnicompress.OmniCompressSession()
    # Big compressible JSON-array content at index 0, + short recent messages
    # (so it falls outside the recent-protect window and actually compresses).
    big = json.dumps([{"id": i, "v": str(i)} for i in range(60)])
    msgs = [{"role": "user", "content": big}]
    msgs += [{"role": "assistant", "content": f"ok {i}"} for i in range(6)]

    res = sess.compress(msgs)
    assert res["tokens_after"] < res["tokens_before"]
    assert len(res["ccr_refs"]) >= 1

    # retrieve must return the original stored by this same session.
    h = res["ccr_refs"][0]["hash"]
    original = sess.retrieve(h)
    assert original is not None
    assert '"id"' in original  # contains part of the original JSON array


def test_session_retrieve_unknown_returns_none():
    sess = omnicompress.OmniCompressSession()
    assert sess.retrieve("deadbeef" * 8) is None
