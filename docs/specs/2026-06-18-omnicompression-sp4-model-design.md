# Spec — OmniCompression · SP4 Modelo OmniCompress

> **Status:** draft v0 — gerado em paralelo (2026-06-18) e auditado por painel externo. Fixes técnicos do audit ainda pendentes (ver `_audit-sp2-sp5-2026-06-18.md`): SP2 precedência `per-call > env > arquivo`; SP3 falha de candidato contabilizada como falha (não passthrough); SP4 `LearnedCompressor` = impl do trait existente + CCR obrigatório antes de compressão abstractiva; SP5 caminho primário = SP1 in-process. **NÃO é spec final — aguardando review.**


> **Origem:** Iniciativa interna OmniForge para vantagem competitiva (moat) em compressão de contexto nativa.
> **Escopo:** Treinar, quantizar e integrar um modelo de compressão de domínio como nova impl do trait `Compressor`.
> **Depende de:** SP1 (Core Determinístico), SP3 (Bench e Dataset).
> **Princípio:** Melhoria opcional e reversível, runtime leve cross-platform, fail-open rigoroso.

## Escopo
**Dentro**
- Treinamento/fine-tuning de modelo de compressão (foco PT-BR e traces de domínio de agentes).
- Quantização (INT8) e exportação para formato ONNX para inferência leve.
- Implementação do `LearnedCompressor` como novo trait `Compressor` no SP1.
- Mecanismo de download opcional e carregamento dinâmico do modelo no runtime (Rust/PyO3).
- Roteamento de falhas e fallback automático para o motor determinístico do SP1.

**Fora**
- Reescrita ou alteração do motor determinístico base do SP1.
- Coleta de novos dados de treinamento (utiliza o dataset consolidado no SP3).
- Inclusão de dependências nativas pesadas no cliente final.

**Não-objetivos explícitos**
- Tornar o modelo aprendido obrigatório para o funcionamento do OmniCompress (o SP1 deve funcionar standalone perfeitamente).
- Substituir a rede CCR do SP1 (o modelo aprendido usará o CCR como rede de segurança para garantir reversibilidade).

## Arquitetura — unidades de fronteira clara

| Unidade | Responsabilidade | Interface (resumo) | Depende de |
| :--- | :--- | :--- | :--- |
| `LearnedCompressor` | Implementa o trait `Compressor` do SP1. Recebe texto e aplica compressão extractiva/abstractiva via modelo. | `fn compress(&self, input: &str) -> Result<CompressedBlock>` | SP1 (`Compressor`), `OnnxRuntime` |
| `ModelManager` | Gerencia o ciclo de vida do modelo: verifica existência do arquivo `.onnx`, carrega pesos na memória e libera recursos. | `fn get_model_instance() -> Option<ModelHandle>` | Arquivo ONNX local |
| `OnnxRuntimeWrapper` | Camada de inferência cross-platform que executa o modelo quantizado usando a crate `ort` nativamente. | `fn run_inference(&self, tokens: &[i64]) -> Vec<i64>` | `ort` crate |
| `TrainingPipeline` | Script offline (Python) que processa o dataset do SP3, treina o modelo PyTorch e exporta a versão quantizada via ONNX. | `fn export_quantized_model()` | Dataset SP3, PyTorch |
| `ModelDownloader` | Módulo auxiliar no binding Python para verificar e baixar o modelo `.onnx` caso o usuário opte por habilitar a feature. | `fn ensure_model_available() -> bool` | Endpoint de download interno |

## Fluxo de dados
```text
[Input: ContextBlock]
      |
      v
[SP1: ContentRouter] ---> Roteia para 'LearnedCompressor' (se modelo disponível e habilitado)
      |
      v
[ModelManager] ---> Carrega modelo ONNX (se ainda não estiver em memória)
      |
      v
[OnnxRuntimeWrapper] ---> Tokeniza -> Executa inferência -> Gera saída comprimida (abstractiva/extractiva)
      |
      v
[SP1: CCRStore] ---> `put(bytes_original)` -> Retorna Hash de recuperação
      |
      v
[CompressionPipeline] ---> Monta mensagem substituta contendo: [Texto Comprimido pelo Modelo] + [CCR Hash Ref]
      |
      v
[Output: CompressResult]
```
*Fluxo de Fallback: Em qualquer erro no `OnnxRuntimeWrapper` ou ausência do `ModelManager`, o `CompressionPipeline` aborta o `LearnedCompressor` e cai para o `LogTextCompressor` ou `PassThrough` do SP1 de forma transparente.*

## Tratamento de erros — fail-open
O SP4 respeita estritamente a regra de fail-open do OmniCompress:
1. **Modelo ausente:** Se o arquivo `.onnx` não estiver no disco (opção de download não executada), `ModelManager` retorna `None`. O `ContentRouter` ignora o `LearnedCompressor` e usa a rota determinística.
2. **Falha de inferência:** Qualquer erro no runtime ONNX (OOM, input mal formado, timeout de CPU) é capturado. A thread de compressão não entra em pânico (`panic`); ela aborta o bloco atual, loga o erro de forma silenciosa em nível de log interno e repassa o bloco original para o `PassThrough` ou compressor determinístico.
3. **Garantia de reversibilidade (CCR):** Ao usar compressão abstractiva do modelo, o texto original é *sempre* enviado para o `CCRStore` antes da substituição na mensagem final. Mesmo que o modelo gere uma abstração imperfeita, o conteúdo original nunca é perdido silenciosamente e pode ser recuperado via hash.

## Estratégia de testes (TDD)
- **Testes Unitários (Rust):**
  - Verificar se `LearnedCompressor` satisfaz o trait `Compressor` corretamente.
  - Testar `ModelManager` forçando ausência do arquivo `.onnx` e validar retorno `None`.
  - Simular um erro fatal injetado no `OnnxRuntimeWrapper` e garantir que a exceção é tratada e o fluxo retorna para `PassThrough`.
- **Testes de Integração:**
  - Iniciar `CompressionPipeline` sem o modelo (verificar comportamento idêntico ao SP1 puro).
  - Iniciar com modelo mockado (fake ONNX) e verificar inclusão de `ccr_refs` no `CompressResult`.
- **Testes de Avaliação (Gate do SP3):**
  - Executar o `EvalHarness` do SP3 usando o `LearnedCompressor` habilitado.
  - Comparar `tokens_saved` e métricas de `accuracy` (retenção semântica) em relação ao baseline determinístico.

## Critérios de sucesso
- **Gate de Integração:** `LearnedCompressor` plugado no SP1 sem regressões no fluxo determinístico. Build cross-platform (Linux, macOS, Windows) sem falhas na compilação do binding Rust/PyO3.
- **Gate de Latência:** Tempo de inferência ONNX INT8 < 100ms por bloco de tamanho médio (até 4k tokens) em uma CPU padrão de mercado.
- **Gate de Eficácia (Promoção a Default Opcional):** O modelo deve demonstrar, no bench do SP3, um aumento de > 15% em economia de tokens em relação ao melhor compressor determinístico do SP1, mantendo a regressão de acurácia das respostas do agente final abaixo de 1%.
- **Gate de Distribuição:** O wheel Python base (sem modelo) não pode ultrapassar o aumento de 2MB no tamanho final. O modelo `.onnx` deve ser baixado apenas mediante comando explícito do usuário.

## Riscos & mitigações

| Risco | Mitigação |
| :--- | :--- |
| Aumento excessivo do tamanho do pacote wheel | Modelo não é empacotado no wheel base. Usa-se um script/método `download_model()` no binding Python para buscar o `.onnx` sob demanda. |
| Latência de CPU inviável para uso em tempo real | Quantização agressiva (INT8). Definição de timeout de inferência no `LearnedCompressor`; se exceder, aborta e usa via determinística. |
| Alucinação do modelo Abstractivo levando à perda de dados críticos | `CCRStore` (SP1) age como rede de segurança. O texto original é sempre armazenado, referenciado por hash na mensagem final, tornando a compressão 100% reversível sob demanda. |
| Incompatibilidade do `ort` em algum SO exótico | `ModelManager` faz check de capacidade no boot. Se não rodar, feature desligada silenciosamente; pipeline usa SP1. |

## Próximos passos
1. Selecionar arquitetura base leve (seq2seq ou encoder-decoder extractivo) para treino no dataset do SP3.
2. Treinar baseline em PyTorch e executar pipeline de exportação e quantização ONNX (INT8).
3. Implementar o esqueleto do `LearnedCompressor` e `ModelManager` em Rust integrando a crate `ort`.
4. Conectar ao `EvalHarness` do SP3 e iterar nos hiperparâmetros de treino até atingir o Gate de Eficácia.