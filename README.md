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
| 🧠 **Cache-aware** | No modo **cache-stable** o prefixo comprimido é **byte-estável entre turnos** (testado ponta-a-ponta): a forma comprimida de um bloco depende só do conteúdo, não da posição, então acrescentar um turno não reescreve nenhuma mensagem anterior e o **cache de prompt do provedor não é invalidado**. O **proxy ativa cache-stable por padrão**; na biblioteca é **opt-in** (`cache_stable=True`), porque o default protege a janela recente. |
| 🔁 **Modo agressivo + CCR (opt-in)** | Para compressão máxima, amostra arrays e elide código/prosa/objetos, guardando o original no **CCR** (Compress-Cache-Retrieve), recuperável por hash. Rende muito mais, mas **exige um loop de retrieve** (ex.: a tool MCP de expandir) — sem ele, queries que precisam do detalhe elidido falham. Por isso **não** é o default. |
| 🔌 **Multi-superfície** | A mesma engine roda como **biblioteca** (Python via PyO3), **proxy HTTP drop-in** (fala OpenAI *e* Anthropic, sem mudar seu código), **servidor MCP** (tools `compress`/`retrieve`/`stats`) e **CLI** — pluga em qualquer fluxo de agente. |
| 🛡️ **Fail-open** | Se um compressor falhar — ou até entrar em pânico — o bloco original **passa intacto**: a request nunca quebra e nenhum dado se perde. Robustez em produção acima de taxa de compressão. |

## Dois modos (escolha honesta)

| Modo | O que faz | Quando usar |
|---|---|---|
| **Lossless** (default) | array/NDJSON → tabela colunar (todas as linhas, schema fatorado, reversível); logs → dedup. Código/prosa/CSV/objeto aninhado passam intactos. **Zero perda, sem retrieve.** | proxy, ou qualquer consumidor sem loop de retrieve |
| **Agressivo** (`lossless=false`) | amostra arrays/NDJSON/CSV, elide código/prosa/objetos; original no CCR. | só com loop de retrieve (ex.: MCP), onde o agente pode expandir |

## Tipos de conteúdo (por que a compressão varia)

O OmniCompress classifica cada bloco e aplica a regra certa. O ganho varia porque a **redundância** varia — só dá pra comprimir o que se repete:

- 📊 **Logs** — linhas quase idênticas repetidas mil vezes → colapso com contagem. Redundância altíssima → **maior ganho**, e lossless (reconstruível).
- 🔢 **JSON / NDJSON / tool-outputs** (resultado de busca, listagem, query) — as mesmas chaves repetidas em toda linha → forma **colunar** fatora o schema uma vez. Vale tanto para arrays JSON quanto para **NDJSON/JSONL** (um objeto por linha), que compartilham o mesmo codec colunar lossless. É o **maior sink de token** dos agentes.
- 📑 **CSV/TSV** — já é colunar, então não há ganho **lossless** (fica intacto, honestamente); no **agressivo** vira header + amostra de linhas, com o original no CCR.
- 💻 **Código** — estrutura via AST (tree-sitter) para **Python, JavaScript, TypeScript, Rust e Go**; corpo de função/método pode ser elidido (só no agressivo, recuperável via CCR). Linguagens fora dessa lista passam **intactas** (fail-open).
- 📝 **Prosa** — texto corrido em linguagem natural (documentação, chat, explicações, e-mail). **Cada palavra carrega significado — não há padrão estrutural pra fatorar.** Por isso comprime pouco e só de forma extractiva (agressivo); no lossless fica **intacta de propósito** (cortar prosa perderia sentido).

**Regra de ouro:** quanto mais estruturado e repetitivo o conteúdo, mais ele comprime **sem perder nada**. Prosa densa é o limite — e é exatamente onde a gente é conservador, não agressivo.

## Resultados (bench reproduzível — **token BPE real**, cl100k via tiktoken)

Redução de **token** (não de caractere) por tipo de conteúdo:

| Conteúdo | Lossless (default) | Agressivo (+retrieve) |
|---|---:|---:|
| Logs repetitivos | **97%** | 97% |
| JSON / NDJSON / tool-outputs | **~33–52%** | **69%** |
| CSV / TSV | 0% (intacto) | amostra + CCR |
| Código (Python · JS · TS · Rust · Go) | 0% (intacto) | até ~58%¹ |
| Prosa | 0% (intacto) | 41% |

¹ Medido em Python; o ganho do código varia por linguagem e tamanho de corpo. Linguagens não suportadas passam intactas.

No lossless, código/prosa/CSV/objeto aninhado passam **intactos** (elidi-los seria lossy); o ganho vem de logs e da forma colunar de arrays/NDJSON — **zero perda, sem retrieve**. O agressivo amostra/elide e guarda o original no CCR (**exige loop de retrieve**).

> Medição honesta: **tokens reais (cl100k)**, não estimativa por caractere — `chars/4` subestimava JSON em ~38%. Rode você mesmo: `omnicompress bench <dir>`. Verificado por harness de acurácia (`eval/`): no lossless o modelo responde **igual ao contexto cheio**. Onde não há ganho real, reportamos **zero** — sem número inflado.

## Como usar

**Biblioteca (Python):**
```python
import omnicompress

# Default: lossless e NÃO cache-stable (a janela recente é protegida).
res = omnicompress.compress(messages)

# Compressão agressiva (amostra/elide; original no CCR) — exige loop de retrieve:
res = omnicompress.compress(messages, lossless=False)

# Front de um cache de prompt? Ative cache-stable para o prefixo ficar byte-estável:
res = omnicompress.compress(messages, cache_stable=True)

# Sessão com CCR persistente (retrieve do original em modo agressivo):
s = omnicompress.OmniCompressSession()
res = s.compress(messages, lossless=False)
s.retrieve(res["ccr_refs"][0]["hash"])
```
Parâmetros opcionais: `lossless` (default `True`), `cache_stable` (default `False`), `protect_recent`, `min_chars_to_compress`.

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
