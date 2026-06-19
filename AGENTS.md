# AGENTS.md — OmniCompress

Guide for agents (and humans) working in this repository.

## What this is

A deterministic, reversible **context-compression layer** for AI agents. See
[`README.md`](README.md) for the product overview and the two compression modes.

## Repository structure

```
crates/omnicompress-core/   # Rust engine: router, compressors, CCR, pipeline, bench
crates/omnicompress-py/     # Python binding (PyO3)
crates/omnicompress-cli/    # CLI: compress | eval | bench
crates/omnicompress-proxy/  # drop-in HTTP proxy (OpenAI + Anthropic wire) + output shaper
crates/omnicompress-mcp/    # MCP server (compress / retrieve / stats)
eval/                       # accuracy harness (Python) — proves "fewer tokens, same answers"
```

## The two modes (read before changing a compressor)

- **Lossless (default, `CompressConfig.lossless = true`):** compressors NEVER drop
  data. A JSON array becomes a columnar table (all rows, shared schema); logs are
  deduped; code/prose/nested objects pass through untouched. Nothing goes to the
  CCR — the model answers from the compressed view alone, no retrieve loop needed.
  This is the safe default for any consumer without a retrieve path (e.g. the proxy).
- **Aggressive (`lossless = false`):** sample arrays, elide code/prose/objects, and
  store the original in the CCR (recoverable by hash). Higher ratio, but the consumer
  **must** wire a retrieve loop (e.g. the MCP tool) or detail-seeking queries will
  fail — see `eval/` for the measured proof of this.

## Development rules

- **Dev flow:** isolated worktree → TDD → quality gate → PR.
- **Clean-room:** the implementation is fully original; do not reference other tools.
- **Fail-open always:** any compression error → pass the original block through. Never lose content.
- **Honest metrics:** exact tokenizer where available; never inflate a ratio. If a change
  affects compression behavior, re-run `eval/` (accuracy harness) and `cargo run -p
  omnicompress-cli -- bench <dir>` and report the real numbers.
- **A lossy step must be gated by `lossless == false`** and must store the original in the CCR.

## Commands

```bash
cargo test --workspace          # Rust tests
cargo clippy --workspace        # lint (keep at 0 warnings)
cargo run -p omnicompress-cli -- bench <dir>   # measure compression
pytest eval/                    # accuracy-harness logic tests (no API key needed)
maturin build -m crates/omnicompress-py/Cargo.toml   # build the Python wheel
```
