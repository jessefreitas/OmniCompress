"""Retrieve-loop logic tests — no LLM, no wheel (MockToolProvider + fake CCR)."""
from retrieve import MockToolProvider, ToolChatProvider


def test_mock_provider_retrieves_then_answers():
    # A compressed marker carrying a CCR hash; the original lives in a fake store.
    store = {"abcd1234ef": "full table: ... checkout region=sa-east-1 ..."}
    compressed = [{"role": "tool", "content": '{"_omnicompress":"json_array"} [omnicompress: hash=abcd1234ef retrieve to expand.]'}]

    mock = MockToolProvider(script=[{"retrieve_hash": "abcd"}, {"answer": "sa-east-1"}])
    mock.retrieve_fn = lambda h: store[h]

    ans, n_ret = mock.answer(compressed, "Which region?")
    assert ans == "sa-east-1"
    assert n_ret == 1  # the model retrieved exactly once


def test_mock_provider_answers_without_retrieve():
    mock = MockToolProvider(script=[{"answer": "42"}])
    mock.retrieve_fn = lambda h: "unused"
    ans, n_ret = mock.answer([{"role": "user", "content": "no marker here"}], "How many?")
    assert ans == "42"
    assert n_ret == 0


def test_retrieve_fn_receives_full_hash_not_substr():
    seen = {}
    store_hash = "deadbeefcafe0001"
    msgs = [{"role": "tool", "content": f"summary [omnicompress: hash={store_hash} retrieve to expand.]"}]

    def rfn(h):
        seen["hash"] = h
        return "ORIGINAL"

    mock = MockToolProvider(script=[{"retrieve_hash": "deadbeef"}, {"answer": "ok"}])
    mock.retrieve_fn = rfn
    mock.answer(msgs, "q?")
    assert seen["hash"] == store_hash  # full hash resolved from the substring, not the substr


def test_sanitize_coerces_tool_role():
    out = ToolChatProvider._sanitize([{"role": "tool", "content": "x"}, {"role": "system", "content": "s"}])
    assert out[0]["role"] == "user"
    assert out[1]["role"] == "system"
