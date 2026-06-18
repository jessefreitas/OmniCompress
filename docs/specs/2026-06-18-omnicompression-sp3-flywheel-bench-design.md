# Spec — OmniCompression · SP3 Flywheel + Bench

> **Status:** draft v0 — gerado em paralelo (2026-06-18) e auditado por painel externo. Fixes técnicos do audit ainda pendentes (ver `_audit-sp2-sp5-2026-06-18.md`): SP2 precedência `per-call > env > arquivo`; SP3 falha de candidato contabilizada como falha (não passthrough); SP4 `LearnedCompressor` = impl do trait existente + CCR obrigatório antes de compressão abstractiva; SP5 caminho primário = SP1 in-process. **NÃO é spec final — aguardando review.**

> **Origem**: SP1 (OmniCompress Core)
> **Escopo (1 linha)**: Infraestrutura de captura de traces, dataset PT-BR versionado e benchmark de compressão/acurácia para validar melhorias no motor.
> **Depende de**: SP1 (OmniCompress Core)
> **Princípio**: Dados honestos, medição dupla (ratio + acurácia) e falha transparente.

## Escopo
**Dentro**:
- Captura de traces de agentes do OmniRift com mecanismo de consentimento e schema rígido.
- Pipeline de anonimização e segurança de dados (PII-safe).
- Construção e versionamento de dataset PT-BR (splits train/val/test, deduplicação, balanceamento por tipo de conteúdo).
- Framework de Benchmark medindo simultaneamente ratio de compressão e preservação de acurácia (reconstrução via CCR e tarefas objetivas).
- Portão lógico de validação ("Candidato Superior") e relatório comparativo entre versões.

**Fora**:
- Treinamento de modelos estatísticos ou neurais de compressão (escopo do SP4).
- Execução e inferência em produção (cobreido pelo SP1).
- Definição do modelo de negócios ou pricing.

**Não-objetivos explícitos**:
- Otimização de prompts ou engenharia de few-shot para tarefas de avaliação.
- Coleta de dados de sistemas de terceiros.

## Arquitetura — unidades de fronteira clara

| Unidade | Responsabilidade | Interface (resumo) | Depende de |
| :--- | :--- | :--- | :--- |
| `TraceCollector` | Capturar payloads reais (tool-output/contexto) dos agentes no OmniRift, respeitando consentimento explícito. | `collect(agent_event) -> Option<RawTrace>` | OmniRift, Schema de Trace |
| `PiiAnonymizer` | Sanitizar e anonimizar dados sensíveis (PII) dos traces brutos. | `sanitize(raw_payload) -> SafePayload` | Regras de Regex/NER PT-BR |
| `DatasetBuilder` | Deduplicar, balancear por tipo de conteúdo (json/code/log/prose/diff) e particionar (train/val/test) versionando o dataset. | `build(safe_traces) -> DatasetVersion` | `PiiAnonymizer` |
| `BenchHarness` | Orquestrar a compressão do dataset usando motores candidatos e o baseline do SP1. | `run(dataset, candidate) -> BenchReport` | SP1 (`CompressionPipeline`, `Tokenizer`) |
| `AccuracyEvaluator` | Medir a perda de acurácia reconstruindo conteúdo via CCR e submetendo a tarefas objetivas com grupo de controle. | `evaluate(orig, comp, ccr_refs) -> AccuracyScore` | SP1 (`CCRStore`), LLM Judge (controlado) |

## Fluxo de dados
```text
[OmniRift Agents] -> (TraceCollector + Consent Flag) -> [Raw Traces]
-> (PiiAnonymizer: fail-safe sanitization) -> [Clean Traces]
-> (DatasetBuilder: dedup/balance/split) -> [Versioned PT-BR Dataset]
-> (BenchHarness + SP1 Pipeline) -> [Compressed Payloads + CCR Refs]
-> (AccuracyEvaluator vs Holdout Control) -> [BenchReport]
-> [SP4 (Treinamento)]
```

## Tratamento de erros — fail-open
- **Anonimização Incerta**: Se o `PiiAnonymizer` não conseguir classificar ou garantir que um bloco está livre de PII, o bloco é descartado. A pipeline não quebra, mas a segurança da dados é absoluta (fail-safe para privacidade, fail-open para continuidade do flywheel).
- **Falha de Reconstrução no Bench**: Se o `AccuracyEvaluator` não conseguir recuperar o conteúdo via `CCRStore`, a amostra é marcada como falha crítica (acurácia = 0), o erro é logado, mas o processamento do dataset continua.
- **Erros de Compressão**: Se um motor candidato falhar, o `BenchHarness` aciona o `PassThrough` do SP1 para garantir que o benchmark não trave.

## Estratégia de testes (TDD)
1. **TraceCollector**: Testar se a captura retorna `None` se a flag de consentimento do agente/instância for `false`.
2. **PiiAnonymizer**: Testar substituição de e-mails, IPs, chaves de API, e formatos PT-BR (CPF, CNPJ, telefones); garantir que conteúdo válido sem PII não é destruído.
3. **DatasetBuilder**: Testar deduplicação por hash exato e verificar se o split de validação contém a proporção correta de `json/code/log/prose/diff`.
4. **BenchHarness**: 
   - Testar se o portão "Candidato Superior" rejeita rigorosamente um motor que melhora o ratio mas derruba a acurácia abaixo do baseline do SP1.
   - Validar que as métricas de tokens usam o tokenizer exato onde disponível e rotulam claramente (~estimado) onde não.

## Critérios de sucesso
- Capacidade de gerar um dataset PT-BR sintético e real contendo pelo menos 10.000 amostras, deduplicadas, balanceadas e com splits bem definidos.
- Geração de um `BenchReport` contendo métricas canônicas: `CompressionRatio` (tokens economizados / tokens originais) e `AccuracyPreservation` (acurácia no conteúdo reconstruído / acurácia no original).
- Presença de um grupo de controle/holdout que prove que os savings são **medidos** (via tokenizer do SP1) e não estimados.
- O Portão de Qualidade valida que um motor só é declarado superior se bater o baseline determinístico do SP1 em ambos os eixos: não regredir acurácia e melhorar (ou manter) o ratio.

## Riscos & mitigações
| Risco | Mitigação |
| :--- | :--- |
| Vazamento de PII no dataset de treino. | Anonimização multi-camada e fail-safe descartando qualquer bloco ambíguo. |
| Dataset enviesado para um tipo de ferramenta de agente. | Balanceamento explícito e quotas estritas no `DatasetBuilder`. |
| Benchmark lento devido à inferência do avaliador de acurácia. | Execução paralela do `BenchHarness` e uso de holdout amostral estatisticamente significativo. |
| Flutuação nas medições por diferenças de tokenizer. | Uso rigoroso das regras de medição honesta definidas no SP1 (exato vs estimado rotulado). |

## Próximos passos
- Consumir o dataset versionado e os relatórios do `BenchHarness` como insumo direto para o SP4, onde o primeiro modelo de compressão PT-BR de domínio será treinado e iterado para bater o portão de "Candidato Superior".