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
| 🧠 **Cache-aware** | The compressed prefix is **byte-stable across turns** (proven by test): a block's compressed form doesn't change as the window slides. So the **provider's prompt cache isn't invalidated** — you don't lose, by re-processing, what you saved by compressing. |
| 🔁 **Aggressive + CCR (opt-in)** | For maximum compression, sample arrays and elide code/prose/objects, storing the original in the **CCR** (Compress-Cache-Retrieve), recoverable by hash. Much higher ratio, but it **requires a retrieve loop** (e.g. the MCP expand tool) — without one, queries needing the elided detail fail. That's why it is **not** the default. |
| 🔌 **Multi-surface** | The same engine runs as a **library** (Python via PyO3), a **drop-in HTTP proxy** (speaks OpenAI *and* Anthropic, no code changes), an **MCP server** (`compress`/`retrieve`/`stats` tools), and a **CLI** — plugs into any agent workflow. |
| 🛡️ **Fail-open** | If a compressor errors — or even panics — the original block **passes through intact**: the request never breaks and no data is lost. Production robustness over compression ratio. |

## Two modes (an honest choice)

| Mode | What it does | When to use |
|---|---|---|
| **Lossless** (default) | array → columnar table (all rows, shared schema); logs → dedup. Code/prose/nested objects pass through intact. **Zero loss, no retrieve.** | a proxy, or any consumer without a retrieve loop |
| **Aggressive** (`lossless=false`) | samples arrays, elides code/prose/objects; original in the CCR. | only with a retrieve loop (e.g. MCP), where the agent can expand |

## Results (real, reproducible benchmark)

**Lossless (default):** compresses the biggest token sink in agent contexts — **array tool outputs** (recall/search/query) — by **~40–70%** with zero loss, and collapses repeated log lines. Code, prose and nested objects pass through **untouched** (eliding them would be lossy).

**Aggressive + retrieve (opt-in):** array **~93%**, code **~84–93%**, prose **~90%**, nested object **~28%** — with the original recoverable via the CCR.

> Honest measurement, verified by an accuracy harness (`eval/`): run it yourself with `omnicompress bench <dir>`. In lossless mode **the model answers the same as with full context** (measured fidelity 100% on a set of detail-seeking queries). Where there's no real gain, we report zero — no inflated numbers.

## Usage

**Library (Python):**
```python
import omnicompress
res = omnicompress.compress(messages)              # one-shot
s = omnicompress.OmniCompressSession()             # with persistent CCR
res = s.compress(messages); s.retrieve(res["ccr_refs"][0]["hash"])
```

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
