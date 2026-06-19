"""Harness logic tests — no LLM, no wheel (MockProvider + stub compressors)."""
import os

from accuracy import Item, default_tokenizer, load_dataset, run, _norm
from providers import MockProvider

HERE = os.path.dirname(__file__)


def region_model(messages, question):
    """Fake model: answers the region only if the fact survives in context."""
    text = " ".join(m.get("content", "") for m in messages)
    return "sa-east-1" if "sa-east-1" in text else "unknown"


def test_load_sample_dataset():
    items = load_dataset(os.path.join(HERE, "sample.jsonl"))
    assert len(items) == 3
    assert items[0].id == "region"
    assert items[0].gold == "sa-east-1"


def test_fidelity_and_accuracy_preserved_when_fact_survives():
    ds = [Item(id="r", context=[{"role": "tool", "content": "x" * 400 + " region sa-east-1 " + "y" * 400}],
               question="region?", gold="sa-east-1")]
    identity = lambda msgs: msgs  # compression keeps the fact
    rep = run(ds, MockProvider(region_model), compress_fn=identity, tokenizer=default_tokenizer)
    assert rep["summary"]["fidelity_rate"] == 1.0
    assert rep["summary"]["accuracy_full"] == 1.0
    assert rep["summary"]["accuracy_comp"] == 1.0


def test_fidelity_and_accuracy_lost_when_fact_dropped():
    ds = [Item(id="r", context=[{"role": "tool", "content": "region sa-east-1 details here"}],
               question="region?", gold="sa-east-1")]
    drop = lambda msgs: [{"role": "tool", "content": "[omitted]"}]  # compression loses the fact
    rep = run(ds, MockProvider(region_model), compress_fn=drop, tokenizer=default_tokenizer)
    assert rep["summary"]["fidelity_rate"] == 0.0     # answer changed -> compression NOT free
    assert rep["summary"]["accuracy_full"] == 1.0
    assert rep["summary"]["accuracy_comp"] == 0.0


def test_compression_ratio_is_measured():
    ds = [Item(id="r", context=[{"role": "tool", "content": "x" * 400}], question="q")]
    halve = lambda msgs: [{"role": "tool", "content": "x" * 200}]
    rep = run(ds, MockProvider(lambda m, q: "ok"), compress_fn=halve, tokenizer=default_tokenizer)
    assert 0 < rep["summary"]["mean_ratio"] < 1.0
    assert rep["summary"]["tokens_saved_pct"] > 0
    assert rep["summary"]["accuracy_full"] is None  # no gold -> accuracy omitted


def test_norm_collapses_whitespace_and_case():
    assert _norm("  SA-East-1\n ") == "sa-east-1"
