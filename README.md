# OmniCompress

> **Camada de compressão de contexto para LLM/agentes** — comprime tool-outputs, logs, JSON, código e histórico **antes de chegar no modelo**. Mesmas respostas, fração dos tokens.

## O que é

Produto **clean-room** da OmniForge — motor e encanamento 100% próprios.

**Diferencial (wedge):**
- **Tecnologia superior** — modelo de compressão PT-BR / de domínio, treinado nos nossos próprios
  traces de agentes.
- **Nativo no OmniRift**.

## Status

🟡 **Fase de design.** O primeiro sub-projeto (SP1 — OmniCompress Core) está especificado em
[`docs/specs/2026-06-18-omnicompression-sp1-core-design.md`](docs/specs/2026-06-18-omnicompression-sp1-core-design.md).
Implementação começa após `writing-plans` + card no Kanban.

## Roadmap

| Sub-projeto | Escopo |
|---|---|
| **SP1 — Core** | Motor determinístico (router + compressores estruturais + CCR + medição) + eval harness. **Spec pronto.** |
| SP2 — Superfícies | Lib pública, proxy OpenAI/Anthropic-compat, MCP server |
| SP3 — Flywheel + Bench | Captura de traces → dataset PT-BR → bench ratio×acurácia |
| SP4 — Modelo | Treina/quantiza o modelo OmniCompress (o moat, medido vs SP3) |
| SP5 — OmniRift | Nativo no OmniRift |

## Stack

- **Núcleo Rust** (`crates/omnicompress-core`) + binding **Python** (PyO3/maturin)
- **Cross-platform**: Linux · macOS · Windows (wheels via CI; CCR embarcado em `redb`, zero processo externo)

## Licença

[Apache 2.0](LICENSE).
