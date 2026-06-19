# Accuracy harness — proving compression is *free*

A compression ratio alone only proves a payload got *smaller*. This harness proves
the claim that matters: **fewer tokens, same answers**.

For each item it runs the question twice — with the **full** context and with the
**OmniCompress-compressed** context — and measures:

- **compression** — how much smaller the context got;
- **fidelity** — did the answer *change* between full and compressed? (the core claim:
  compression that doesn't change the answer is free);
- **accuracy** (when a `gold` answer is provided) — correctness with full vs compressed
  context, side by side.

## Dataset format (JSONL)

One JSON object per line:

```json
{"id": "region", "context": [{"role": "tool", "content": "...big tool output..."}], "question": "Which region?", "gold": "sa-east-1"}
```

`gold` is optional. With it you get accuracy; without it you still get fidelity + ratio.
A small `sample.jsonl` is included for a smoke run, and any QA-over-context dataset
(e.g. LongMemEval, exported to this shape) works.

## Running

Needs the built `omnicompress` wheel (`maturin develop` in `crates/omnicompress-py`)
and any OpenAI-compatible endpoint (OpenAI, a local model, or the OmniCompress proxy).

```bash
export OMNICOMPRESS_EVAL_API_KEY=sk-...
python eval/accuracy.py eval/sample.jsonl \
    --base-url https://api.openai.com/v1 \
    --model gpt-4o-mini \
    --report-out eval-report.json
```

It writes `eval-report.json` (per-item + summary) and prints a Markdown table ending in
the headline: **"X% fewer tokens, fidelity Y%."**

## How it stays honest

- **Reproducible:** you run it on *your own* data and read the numbers yourself.
- **Fidelity over cherry-picking:** it reports when compression *did* change an answer,
  not just the wins.
- **No inflated ratios:** the same tokenizer measures both sides.

## Tests

`pytest eval/` exercises the harness logic with a mock model and stub compressors
(no API key, no wheel needed) — including the case where compression drops a fact and
fidelity correctly drops to 0.
