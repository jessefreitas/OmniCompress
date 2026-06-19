<div align="center">

> 🌐 **English** · [Português](README.md)

# OmniCompress

**The context-compression layer for AI agents.**
Fewer tokens, same answers — lossless, no model in the loop, without breaking your cache.

</div>

---

## The problem

AI agents drown the model in tokens: huge tool outputs, logs, JSON, code, and a history that only grows. That's expensive and blows the context window.

The usual approaches fix **only one piece** and charge a price:
- they **drop** information (lossy — when the agent needs the detail, it's gone);
- they run **a model** to compress (slow, with inference cost — you pay tokens to save tokens);
- they **break the provider's prompt cache** (what you save by compressing, you lose re-processing);
- they cover **a single content type** (shell only, prose only, history only).

## What OmniCompress is

The **unified layer**: it compresses everything that reaches the model — tool outputs, JSON, code, logs, prose, history — and everything the model writes back. It runs **locally**, is **deterministic**, and is **reversible**.

```
your agent  →  [ OmniCompress compresses here ]  →  LLM (Anthropic · OpenAI · …)
                 deterministic · reversible · local
```

## Why it's different

| Principle | What it means |
|---|---|
| 🟢 **Lossless by default** | In the default mode **nothing is dropped**: an array of objects becomes a **columnar table** (the schema is factored out once + every row follows as a value-tuple), and identical log lines collapse with a count. The compressed form carries **100% of the data** — the model answers from what it sees, with **no retrieve round-trip**. It's the safe default for any consumer, including a passive proxy. |
| ⚡ **Deterministic** | Compression is **pure algorithm** — statistics + AST (tree-sitter), **not an ML model in the hot path**. Same input → same output, in **milliseconds** and at **zero inference cost**: you don't pay tokens to save tokens, nor add the latency of a second call. |
| 🧠 **Cache-aware** | In **cache-stable** mode the compressed prefix is **byte-stable across turns** (tested end-to-end): a block's compressed form depends only on its content, not its position, so appending a turn never rewrites an earlier message and the **provider's prompt cache isn't invalidated**. The **proxy enables cache-stable by default**; in the library it's **opt-in** (`cache_stable=True`), because the default protects the recent window. |
| 🔁 **Aggressive + CCR (opt-in)** | For maximum compression, sample arrays and elide code/prose/objects, storing the original in the **CCR** (Compress-Cache-Retrieve), recoverable by hash. Much higher ratio, but it **requires a retrieve loop** (e.g. the MCP expand tool) — without one, queries needing the elided detail fail. That's why it is **not** the default. |
| 🔌 **Multi-surface** | The same engine runs as a **library** (Python via PyO3), a **drop-in HTTP proxy** (speaks OpenAI *and* Anthropic, no code changes), an **MCP server** (`compress`/`retrieve`/`stats` tools), and a **CLI** — plugs into any agent workflow. |
| 🛡️ **Fail-open** | If a compressor errors — or even panics — the original block **passes through intact**: the request never breaks and no data is lost. Production robustness over compression ratio. |

## Two modes (an honest choice)

| Mode | What it does | When to use |
|---|---|---|
| **Lossless** (default) | array/NDJSON → columnar table (all rows, shared schema, reversible); logs → dedup. Code/prose/CSV/nested objects pass through intact. **Zero loss, no retrieve.** | a proxy, or any consumer without a retrieve loop |
| **Aggressive** (`lossless=false`) | samples arrays/NDJSON/CSV, elides code/prose/objects; original in the CCR. | only with a retrieve loop (e.g. MCP), where the agent can expand |

## Content types (why compression varies)

OmniCompress classifies each block and applies the right rule. The gain varies because **redundancy** varies — you can only compress what repeats:

- 📊 **Logs** — near-identical lines repeated thousands of times → collapse with a count. Highest redundancy → **biggest gain**, and lossless (reconstructable).
- 🔢 **JSON / NDJSON / tool outputs** (search results, listings, queries) — the same keys repeated on every row → a **columnar** form factors the schema out once. Applies to JSON arrays and **NDJSON/JSONL** (one object per line) alike — they share the same lossless columnar codec. This is the **biggest token sink** in agent contexts.
- 📑 **CSV/TSV** — already columnar, so there's no **lossless** gain (left intact, honestly); in **aggressive** mode it becomes a header + a row sample, original in the CCR.
- 💻 **Code** — structure via AST (tree-sitter) for **Python, JavaScript, TypeScript, Rust, and Go**; function/method bodies can be elided (aggressive mode only, recoverable via CCR). Languages outside that list pass through **intact** (fail-open).
- 📝 **Prose** — running natural-language text (docs, chat, explanations, email). **Every word carries meaning — there's no structural pattern to factor out.** So it compresses little, and only extractively (aggressive); in lossless it's left **untouched on purpose** (cutting prose would lose meaning).

**Rule of thumb:** the more structured and repetitive the content, the more it compresses **with zero loss**. Dense prose is the limit — and exactly where we stay conservative, not aggressive.

## Results (reproducible benchmark — **real BPE tokens**, cl100k via tiktoken)

**Token** reduction (not character) by content type:

| Content | Lossless (default) | Aggressive (+retrieve) |
|---|---:|---:|
| Repetitive logs | **97%** | 97% |
| JSON / NDJSON / tool outputs | **~33–52%** | **69%** |
| CSV / TSV | 0% (untouched) | sample + CCR |
| Code (Python · JS · TS · Rust · Go) | 0% (untouched) | up to ~58%¹ |
| Prose | 0% (untouched) | 41% |

¹ Measured on Python; the code gain varies by language and body size. Unsupported languages pass through untouched.

In lossless mode code/prose/CSV/nested objects pass through **untouched** (eliding them would be lossy); the gains come from logs and the columnar array/NDJSON form — **zero loss, no retrieve**. Aggressive samples/elides and stores the original in the CCR (**requires a retrieve loop**).

> Honest measurement: **real tokens (cl100k)**, not a per-character estimate — `chars/4` undercounted JSON by ~38%. Run it yourself: `omnicompress bench <dir>`. Verified by an accuracy harness (`eval/`): in lossless mode the model answers **the same as with full context**. Where there's no real gain, we report **zero** — no inflated numbers.

## Usage

**Library (Python):**
```python
import omnicompress

# Default: lossless and NOT cache-stable (the recent window is protected).
res = omnicompress.compress(messages)

# Aggressive compression (sample/elide; original in the CCR) — needs a retrieve loop:
res = omnicompress.compress(messages, lossless=False)

# Fronting a prompt cache? Enable cache-stable for a byte-stable prefix:
res = omnicompress.compress(messages, cache_stable=True)

# Session with a persistent CCR (retrieve the original in aggressive mode):
s = omnicompress.OmniCompressSession()
res = s.compress(messages, lossless=False)
s.retrieve(res["ccr_refs"][0]["hash"])
```
Optional params: `lossless` (default `True`), `cache_stable` (default `False`), `protect_recent`, `min_chars_to_compress`.

**Drop-in proxy** (no code changes):
```bash
OMNICOMPRESS_UPSTREAM=https://api.openai.com omnicompress-proxy   # 127.0.0.1:8787
```

**MCP server:** `omnicompress-mcp` (tools `omnicompress_compress` / `_retrieve` / `_stats`).
**CLI:** `omnicompress compress|eval|bench`.

## Stack

**Rust** core (`omnicompress-core`) + **Python** binding (PyO3) + `proxy`, `mcp`, `cli` crates.
Cross-platform (Linux · macOS · Windows). CCR embedded in `redb` — zero external process.

## Status

Early (v0.x). API may change. Honest feedback welcome.

## License

[Apache 2.0](LICENSE).
