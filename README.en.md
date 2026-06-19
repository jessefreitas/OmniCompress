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
| 🔁 **Reversible (CCR)** | the original goes to the Compress-Cache-Retrieve store and comes back by hash on demand. **Lossless-with-retrieval — never loses data.** |
| ⚡ **Deterministic** | algorithmic compression (statistics + AST), **no model in the hot path**. Zero inference cost, milliseconds. |
| 🧠 **Cache-aware** | byte-stable prefix across turns — it **doesn't invalidate the provider's prompt cache** (proven by test). |
| 📦 **Every content type** | JSON (arrays and nested objects), code (tree-sitter AST), logs, prose — not a single niche. |
| ✍️ **Input *and* output** | shrinks what you send **and** steers the model to write leaner (output costs up to 5× on Opus). |
| 🔌 **Multi-surface** | library, drop-in HTTP proxy, MCP server, and CLI — plugs into any workflow. |
| 🛡️ **Fail-open** | a compression error never fails the request or loses content — it passes through intact. |

None of these is new on its own. **Combining all seven in a single layer is the differentiator.**

## Results (real, reproducible benchmark)

| Content | Reduction |
|---|---:|
| Array tool output (recall/search/query) | **~93%** |
| Code (AST) | **~84–93%** |
| Prose | **~90%** |
| Nested / config JSON | **~28%** |

> Honest measurement: run it yourself with `omnicompress bench <dir>`. Where there's no real gain, we report zero — no inflated numbers.

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
