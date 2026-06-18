# Spec — OmniCompression · SP1 OmniCompress Core

> **Origem:** decisão de produto OmniForge (brainstorming 2026-06-18) — construir uma camada
> própria de compressão de contexto para LLM/agentes, que corte custo de token no dispatch e seja
> nativa no OmniRift.
> **Produto:** OmniCompression = produto novo, **clean-room** (motor + encanamento 100% próprios).
> **Wedge:** (1) **tecnologia superior** — modelo de compressão PT-BR/domínio treinado nos nossos
> próprios traces de agentes; (2) **nativo no OmniRift**.
> **Princípio:** value-first (dogfood já no SP1), cross-platform desde o dia 1, fail-open
> (nunca perde conteúdo).

## Decisões travadas no brainstorming (2026-06-18)

| Eixo | Decisão | Por quê |
|---|---|---|
| Natureza | Produto novo | Jesse |
| Wedge | Tech superior + nativo no OmniRift | Jesse (multi-select) |
| Código | Clean-room próprio | controle total de IP/marca |
| Stack SP1 | **Rust core + Python** (PyO3/maturin) | perf + maturidade de runtime |
| CCR store default | **KV embarcado em Rust (`redb`)** | cross-platform sem fricção (Windows não tem Redis nativo); Redis = backend opt-in |
| Escopo SP1 | Motor determinístico (sem ML) + eval harness, dogfood | fundação + valor imediato + pré-requisito do moat |

## Por que SP1 primeiro (decomposição)

OmniCompression é produto de vários trimestres. Decomposto em:

```
SP1 ▸ OmniCompress Core   motor determinístico + CCR + medição + eval harness   ← ESTE SPEC
SP2 ▸ Superfícies         lib pública, proxy OpenAI/Anthropic-compat, MCP server
SP3 ▸ Flywheel + Bench    captura de traces → dataset PT-BR → bench ratio×acurácia
SP4 ▸ Modelo OmniCompress treina/quantiza o modelo PT-BR de domínio (o MOAT, medido vs SP3)
SP5 ▸ OmniRift            nativo no OmniRift
```

SP1 entrega valor já (corta custo de dispatch — dogfood), fixa a arquitetura que tudo estende, e **constrói o eval harness que é pré-requisito de qualquer alegação de "superior"** (não dá pra provar moat sem harness + dados primeiro).

---

## Escopo SP1

**Dentro:**
- Motor de compressão **determinístico** (sem modelo ML): router + compressores estruturais + CCR reversível + medição honesta.
- Empacotado como **crate Rust** + **binding Python** (PyO3/maturin) com wheels cross-platform.
- **Eval harness** rodando corpus real de payloads.
- **CLI mínima** (`omnicompress compress <file>`, `omnicompress eval`) pra dogfood.
- Integração de dogfood: chamável via lib, medindo ganho real.

**Fora (SPs futuros):** modelo ML/PT-BR (SP4), proxy HTTP + MCP server (SP2), captura de traces/bench formal (SP3), integração nativa no OmniRift (SP5), output-token shaping (SP2+).

**Não-objetivos explícitos:** não duplicar camada de memória persistente existente; SP1 é 100% local/offline.

---

## Arquitetura — unidades de fronteira clara

| Unidade | Responsabilidade | Interface (resumo) | Depende de |
|---|---|---|---|
| `Tokenizer` | Contagem de tokens por provedor | `count(text|messages) -> usize` | — |
| `ContentRouter` | Classifica bloco por **conteúdo + proveniência** (não por nome de tool) | `route(block) -> ContentKind` | Tokenizer |
| `Compressor` (trait) | Comprime um bloco de um tipo | `compress(block) -> (compressed, CcrRef?)` | CCRStore |
| ├ `JsonCrusher` | Arrays-de-dicts/tabelas → schema + amostra + stats | impl de `Compressor` | — |
| ├ `CodeCompressor` | AST (tree-sitter): mantém imports/assinaturas/tipos, corpo → ref CCR | impl de `Compressor` | tree-sitter |
| ├ `LogTextCompressor` | Dedup de linhas, colapso de repetição, extractive (sem ML) | impl de `Compressor` | — |
| └ `PassThrough` | Conteúdo protegido — não toca | impl de `Compressor` | — |
| `CCRStore` (trait) | Reversibilidade: guarda original, devolve por hash | `put(bytes)->Hash`, `get(Hash)->Option<bytes>` | — |
| ├ `EmbeddedStore` (**default**) | `redb` on-disk, durável, cross-platform, TTL configurável | impl de `CCRStore` | redb |
| ├ `MemoryStore` | in-process (teste/zero-dep) | impl de `CCRStore` | — |
| └ `RedisStore` (opt-in) | deploy servidor/compartilhado (Linux) | impl de `CCRStore` | redis |
| `ProtectionPolicy` | O que nunca comprimir (recentes N, conteúdo crítico de edição) | `is_protected(block, ctx) -> bool` | — |
| `CompressionPipeline` | Orquestra: protege → roteia → comprime → grava CCR → remonta | `compress(messages, cfg) -> CompressResult` | todas |
| `Measurement` | Contabilidade + savings honesto (só input no SP1) | embutido no `CompressResult` | Tokenizer |
| `EvalHarness` | Roda corpus → ratio por tipo + round-trip CCR + A/B opcional | `eval(corpus) -> EvalReport` | Pipeline, CCRStore |

`CompressResult { messages, tokens_before, tokens_after, tokens_saved, transforms[], ccr_refs[] }`.

**Camadas:** trait core 100% Rust (`crates/omnicompress-core`); binding Python fino (`crates/omnicompress-py` via PyO3) expõe `compress(messages, **cfg) -> CompressResult`. A CLI é um bin Rust.

---

## Fluxo de dados

```
messages
  → CompressionPipeline
      para cada bloco:
        ProtectionPolicy.is_protected? ──sim→ PassThrough
              │não
        ContentRouter.route → ContentKind
        Compressor[kind].compress → (compressed, CcrRef?)
        se CcrRef: CCRStore.put(original) → hash; injeta marcador de retrieve
  → CompressResult (messages comprimidos + refs + métricas)

reverso (sob demanda do LLM): retrieve(hash) → CCRStore.get(hash) → original byte-idêntico
```

---

## Contrato de reversibilidade (CCR)

- Todo bloco comprimido com perda carrega um **hash estável**; `get(hash)` devolve o original
  **byte-idêntico** enquanto dentro do TTL.
- **Durabilidade (princípio de design):** cache de CCR puramente em RAM com TTL converte
  silenciosamente "lossless com retrieval" em "lossy" ao expirar — é o elo fraco da garantia de
  "sem perda de acurácia". Por isso o default `EmbeddedStore` (`redb`) é **on-disk durável**
  (sobrevive restart), capacidade = disco, e **expiração nunca vira perda silenciosa**: ao
  expirar/evictar, o marcador de retrieve é reescrito como "original descartado (expirado)" —
  explícito, auditável, nunca lossy-silencioso.
- TTL e tamanho máximo são configuráveis; default conservador (ex: 24h on-disk).

---

## Cross-platform (Linux · macOS · Windows)

| Componente | Linux | macOS (Intel+ARM) | Windows | Nota |
|---|---|---|---|---|
| Rust core (PyO3/maturin) | ✅ | ✅ | ✅ | wheels por plataforma via `maturin-action`/`cibuildwheel` no CI |
| Python ≥3.10 | ✅ | ✅ | ✅ | — |
| tree-sitter (crate Rust) | ✅ | ✅ | ✅ | grammars compiladas no build do crate |
| `EmbeddedStore` (`redb`) | ✅ | ✅ | ✅ | **puro Rust, single-file, zero processo externo** |
| `RedisStore` (opt-in) | ✅ | ✅ | ⚠️ | Windows só via WSL2/Memurai/Docker → por isso é opt-in, não default |

**Regra de portabilidade:** o caminho default (lib + EmbeddedStore) tem **zero dependência externa** —
instala via `pip install omnicompress` (wheel) e roda igual nos 3 SOs. Nada de exigir
Redis/Docker/WSL no cliente. CI obrigatório constrói e testa wheels em `ubuntu-latest`,
`macos-latest`, `windows-latest` (matriz x86_64 + arm64).

---

## Tratamento de erros — fail-open

Qualquer erro de compressor/router/CCR → **passthrough daquele bloco** (conteúdo original intacto)
+ log estruturado + métrica `omnicompress_fallback_total{reason}`. Nunca falha a request por causa
de compressão; nunca perde conteúdo.

---

## Medição honesta

- `tokens_before/after/saved` por request, via `Tokenizer`.
- **Honestidade do tokenizer:** exato onde há tokenizer local (OpenAI/tiktoken, Ollama/HF). Para
  **Anthropic não há tokenizer local público** → usamos aproximação **calibrada** e rotulamos a
  métrica como `~estimado` (nunca número inventado apresentado como exato).
- SP1 mede só **input** (output-shaper é SP2+).

---

## Estratégia de testes (TDD)

- **Unit por compressor:** ratio mínimo por tipo + **reconstrução** (round-trip via CCR) byte-idêntica.
- **Router:** tabela de classificação (json/code/log/prose/diff/unknown) com fixtures reais.
- **ProtectionPolicy:** recentes-N e conteúdo crítico nunca comprimidos.
- **CCRStore:** round-trip, TTL/expiração → marcador honesto (não lossy-silencioso), os 3 backends contra o mesmo conjunto de testes de contrato.
- **Pipeline:** integração end-to-end numa sessão misturada.
- **Fail-open:** compressor que lança → bloco volta intacto, métrica incrementada.
- **Cross-platform:** suíte roda na matriz CI (3 SOs).
- **Eval harness:** corpus real, relatório de ratio por tipo + fidelidade.

---

## Critérios de sucesso (dogfood)

1. Chamável via lib Python.
2. No corpus real (sessão misturada): **redução blended ≥ 68%** (meta de benchmark interno) com compressores 100% nossos.
3. **100% de fidelidade** no round-trip CCR (zero perda dentro do TTL).
4. **Zero perda de conteúdo** no fail-open.
5. **Sem custo de ML** (SP1 é determinístico) → import e cold-start rápidos.
6. Wheels verdes nos 3 SOs no CI.

---

## Riscos & mitigações

| Risco | Mitigação |
|---|---|
| "Tech superior" não se prova sem modelo (SP4) | SP1 entrega paridade estrutural + **o harness** que torna o SP4 provável; não overclaim antes do SP4 |
| Build cross-platform de Rust+tree-sitter é chato | `maturin-action` + matriz CI desde o 1º commit; tree-sitter via crate (sem toolchain C manual) |
| `redb` imaturo vs SQLite | trait `CCRStore` desacopla; se `redb` decepcionar, trocar por `rusqlite` é uma impl — testes de contrato garantem |
| Tokenizer Anthropic sem fonte local | métrica rotulada `~estimado` + calibração; honestidade > número bonito |
| Escopo inchar pra SP2-5 | não-objetivos explícitos acima; proxy/MCP/modelo/OmniRift são SPs separados |

---

## Próximos passos (pós-aprovação deste spec)

1. Review do Jesse neste spec.
2. `writing-plans` → plano de implementação detalhado do SP1 (TDD, ordem de unidades).
3. Card no Kanban (funnel 5) antes de qualquer código (regra Kanban-First).
