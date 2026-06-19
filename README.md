<div align="center">

> 🌐 [English](README.en.md) · **Português**

# OmniCompress

**A camada de compressão de contexto para agentes de IA.**
Menos tokens, mesmas respostas — sem perder nada, sem rodar modelo, sem quebrar seu cache.

</div>

---

## O problema

Agentes de IA afogam o modelo em tokens: tool-outputs gigantes, logs, JSON, código, histórico que cresce sem parar. Isso custa caro e estoura a janela de contexto.

As abordagens comuns resolvem **só um pedaço** e cobram um preço:
- ou **descartam** informação (lossy — quando o agente precisa do detalhe, já era);
- ou rodam **um modelo** pra comprimir (lento e com custo de inferência — você paga token pra economizar token);
- ou **quebram o cache de prompt** do provedor (e o que você economiza comprimindo, perde re-processando);
- ou cobrem **um tipo de conteúdo só** (só shell, só prosa, só histórico).

## O que é o OmniCompress

A **camada unificada**: comprime tudo que chega ao modelo — tool-outputs, JSON, código, logs, prosa, histórico — e tudo que o modelo escreve de volta. Roda **local**, é **determinístico** e é **reversível**.

```
seu agente  →  [ OmniCompress comprime aqui ]  →  LLM (Anthropic · OpenAI · …)
                 determinístico · reversível · local
```

## Por que ele é diferente

| Princípio | O que significa |
|---|---|
| 🔁 **Reversível (CCR)** | o original vai pro Compress-Cache-Retrieve e volta por hash sob demanda. **Lossless com retrieval — nunca perde dado.** |
| ⚡ **Determinístico** | compressão por algoritmo (estatística + AST), **sem modelo no caminho quente**. Zero custo de inferência, milissegundos. |
| 🧠 **Cache-aware** | prefixo byte-estável entre turnos — **não invalida o cache de prompt** do provedor (provado por teste). |
| 📦 **Todo conteúdo** | JSON (array e objeto aninhado), código (AST tree-sitter), logs, prosa — não um nicho só. |
| ✍️ **Input *e* output** | encolhe o que você manda **e** orienta o modelo a escrever mais enxuto (output custa até 5× no Opus). |
| 🔌 **Multi-superfície** | biblioteca, proxy HTTP drop-in, servidor MCP e CLI — pluga em qualquer fluxo. |
| 🛡️ **Fail-open** | erro de compressão nunca falha a request nem perde conteúdo — passa intacto. |

Nenhuma dessas, sozinha, é nova. **A combinação das sete numa camada só é o diferencial.**

## Resultados (bench real, reproduzível)

| Conteúdo | Redução |
|---|---:|
| Tool-output em array (recall/busca/query) | **~93%** |
| Código (AST) | **~84–93%** |
| Prosa | **~90%** |
| JSON aninhado / config | **~28%** |

> Medição honesta: rode você mesmo com `omnicompress bench <dir>`. Onde não há ganho real, reportamos zero — sem número inflado.

## Como usar

**Biblioteca (Python):**
```python
import omnicompress
res = omnicompress.compress(messages)              # one-shot
s = omnicompress.OmniCompressSession()             # com CCR persistente
res = s.compress(messages); s.retrieve(res["ccr_refs"][0]["hash"])
```

**Proxy drop-in** (sem mudar seu código):
```bash
OMNICOMPRESS_UPSTREAM=https://api.openai.com omnicompress-proxy   # 127.0.0.1:8787
```

**MCP server:** `omnicompress-mcp` (tools `omnicompress_compress` / `_retrieve` / `_stats`).
**CLI:** `omnicompress compress|eval|bench`.

## Stack

Núcleo **Rust** (`omnicompress-core`) + binding **Python** (PyO3) + crates `proxy`, `mcp`, `cli`.
Cross-platform (Linux · macOS · Windows). CCR embarcado em `redb` — zero processo externo.

## Status

Início (v0.x). API pode mudar. Feedback honesto é bem-vindo.

## Licença

[Apache 2.0](LICENSE).
