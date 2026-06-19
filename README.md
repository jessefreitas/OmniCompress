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
| 🟢 **Lossless por padrão** | no modo default, nada é descartado — o conteúdo comprimido contém todos os dados (array vira tabela colunar). **O modelo responde só com o que vê, sem precisar de retrieve.** |
| ⚡ **Determinístico** | compressão por algoritmo (estatística + AST), **sem modelo no caminho quente**. Zero custo de inferência, milissegundos. |
| 🧠 **Cache-aware** | prefixo byte-estável entre turnos — **não invalida o cache de prompt** do provedor (provado por teste). |
| 🔁 **Modo agressivo + CCR (opt-in)** | pra compressão máxima, elide/amostra e guarda o original no Compress-Cache-Retrieve (volta por hash). **Exige um loop de retrieve** — use só onde o agente pode chamar a tool de expandir. |
| 🔌 **Multi-superfície** | biblioteca, proxy HTTP drop-in, servidor MCP e CLI — pluga em qualquer fluxo. |
| 🛡️ **Fail-open** | erro de compressão nunca falha a request nem perde conteúdo — passa intacto. |

## Dois modos (escolha honesta)

| Modo | O que faz | Quando usar |
|---|---|---|
| **Lossless** (default) | array → tabela colunar (todas as linhas, schema fatorado); logs → dedup. Código/prosa/objeto aninhado passam intactos. **Zero perda, sem retrieve.** | proxy, ou qualquer consumidor sem loop de retrieve |
| **Agressivo** (`lossless=false`) | amostra arrays, elide código/prosa/objetos; original no CCR. | só com loop de retrieve (ex.: MCP), onde o agente pode expandir |

## Resultados (bench real, reproduzível)

**Lossless (default):** comprime o maior sink de tokens dos agentes — **tool-outputs em array** (recall/busca/query) — em **~40–70%** sem perder nada, e colapsa linhas de log repetidas. Código, prosa e objetos aninhados passam **intactos** (elidi-los seria lossy).

**Agressivo + retrieve (opt-in):** array **~93%**, código **~84–93%**, prosa **~90%**, objeto aninhado **~28%** — com o original recuperável via CCR.

> Medição honesta e verificada por harness de acurácia (`eval/`): rode você mesmo com `omnicompress bench <dir>`. No lossless, **o modelo responde igual ao contexto cheio** (fidelidade medida 100% num conjunto de queries que exigem o detalhe). Onde não há ganho real, reportamos zero — sem número inflado.

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
