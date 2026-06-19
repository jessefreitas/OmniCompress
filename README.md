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
| 🟢 **Lossless por padrão** | No modo default **nada é descartado**: uma array de objetos vira uma **tabela colunar** (o schema é fatorado uma vez + todas as linhas seguem como tuplas de valores), e linhas de log idênticas são colapsadas com contagem. O conteúdo comprimido carrega **100% dos dados** — o modelo responde só com o que vê, **sem nenhum round-trip de retrieve**. É o default seguro pra qualquer consumidor, inclusive um proxy passivo. |
| ⚡ **Determinístico** | A compressão é **algoritmo puro** — estatística + AST (tree-sitter), **não um modelo de ML no caminho quente**. Mesma entrada → mesma saída, em **milissegundos** e com **custo zero de inferência**: você não paga token pra economizar token, nem adiciona a latência de uma segunda chamada. |
| 🧠 **Cache-aware** | O prefixo comprimido é **byte-estável entre turnos** (provado por teste): a forma comprimida de um bloco não muda conforme a janela desliza. Assim o **cache de prompt do provedor não é invalidado** — você não perde, re-processando, o que economizou comprimindo. |
| 🔁 **Modo agressivo + CCR (opt-in)** | Para compressão máxima, amostra arrays e elide código/prosa/objetos, guardando o original no **CCR** (Compress-Cache-Retrieve), recuperável por hash. Rende muito mais, mas **exige um loop de retrieve** (ex.: a tool MCP de expandir) — sem ele, queries que precisam do detalhe elidido falham. Por isso **não** é o default. |
| 🔌 **Multi-superfície** | A mesma engine roda como **biblioteca** (Python via PyO3), **proxy HTTP drop-in** (fala OpenAI *e* Anthropic, sem mudar seu código), **servidor MCP** (tools `compress`/`retrieve`/`stats`) e **CLI** — pluga em qualquer fluxo de agente. |
| 🛡️ **Fail-open** | Se um compressor falhar — ou até entrar em pânico — o bloco original **passa intacto**: a request nunca quebra e nenhum dado se perde. Robustez em produção acima de taxa de compressão. |

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
