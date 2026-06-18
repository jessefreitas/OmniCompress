# Spec — OmniCompression · SP5 OmniRift

> **Status:** draft v0 — gerado em paralelo (2026-06-18) e auditado por painel externo. Fixes técnicos do audit ainda pendentes (ver `_audit-sp2-sp5-2026-06-18.md`): SP2 precedência `per-call > env > arquivo`; SP3 falha de candidato contabilizada como falha (não passthrough); SP4 `LearnedCompressor` = impl do trait existente + CCR obrigatório antes de compressão abstractiva; SP5 caminho primário = SP1 in-process. **NÃO é spec final — aguardando review.**


> **Origem**: OmniForge
> **Escopo (1 linha)**: Tornar o OmniCompress nativo no OmniRift, permitindo compressão de contexto transparente e configurável por agente no canvas de orquestração.
> **Depende de**: SP1 (OmniCompress Core), SP2 (Superfíce de Proxy/MCP), OmniRift Core
> **Princípio**: Integração transparente via hooks nativos, isolamento absoluto de estado entre agentes e medição honesta de economia.

## Escopo

**Dentro:**
- Configuração de compressão por agente (ligar/desligar, perfil, TTL do CCR).
- Hook (`RiftInterceptor`) no ciclo de vida do agente no OmniRift para pré-processar contexto via SP1/SP2.
- Componente de UX (`SavingsDashboard`) exibindo economia de tokens e acesso ao `retrieve` do CCR.
- Mecanismo de isolamento de cache (`CCRSandbox`) garantindo que a compressão de um agente não vaze no contexto de outro.

**Fora:**
- Reimplementação do motor de compressão ou tokenizadores (responsabilidade do SP1).
- Reescrita da camada de rede ou proxy (responsabilidade do SP2).
- Modificação do motor de execução de agentes do OmniRift, apenas interceptação de seu fluxo de dados.

**Não-objetivos explícitos:**
- Compressão de prompts de sistema imutáveis definidos estrategicamente pelo criador do agente.
- Compartilhamento de cache CCR entre agentes (cada agente deve operar em seu próprio escopo isolado e reversível).

## Arquitetura — unidades de fronteira clara

| Unidade | Responsabilidade | Interface (resumo) | Depende de |
| :--- | :--- | :--- | :--- |
| `AgentCompressionConfig` | Gerenciar preferências de compressão por nó/agente no canvas (toggle, perfil de compressão, TTL). | Struct de config serializada anexada ao estado do agente no OmniRift. | OmniRift Core |
| `RiftInterceptor` | Interceptar o payload de contexto antes do envio ao LLM, ler a config e rotear para compressão. | `async fn intercept(messages: Vec<Message>) -> Vec<Message>` | SP1, SP2, OmniRift Core |
| `CCRSandbox` | Prover isolamento baseado em namespace para o `CCRStore`, garantindo que as hashes e originais de um agente não sejam acessíveis por outro. | `fn get_context(agent_id: Uuid, hash: Hash) -> Option<bytes>` | SP1 (`CCRStore`) |
| `SavingsDashboard` | Renderizar na UI do OmniRift as métricas do `CompressResult` (tokens salvos, origem) e botão de "Recuperar Original". | Componente de UI que assina eventos de ciclo do `RiftInterceptor`. | OmniRift Core (UI), SP1 (`Measurement`) |

## Fluxo de dados

```text
[OmniRift Canvas] -> [Agente solicita ação com contexto atual]
  -> [RiftInterceptor intercepta pacote de mensagens]
  -> [Verifica AgentCompressionConfig]
    -> SE OFF: [Encaminha pacote original ao LLM (Passthrough)]
    -> SE ON:
       -> [Solicita compressão via SP2/SP1]
       -> [CompressionPipeline.compress()]
       -> [CCRSandbox armazena conteúdo original isolado por agent_id]
       -> [Retorna pacote comprimido]
  -> [Mensagens enviadas ao LLM]
  -> [Resposta do LLM recebida]
  -> [SavingsDashboard atualiza estado com tokens_before/after/saved]
  -> [Ciclo do agente continua]
```

## Tratamento de erros — fail-open

A regra de fail-open é estritamente preservada na camada de integração. Se o `RiftInterceptor` encontrar uma falha (ex: `CompressionPipeline` panic, `CCRSandbox` indisponível, erro de I/O), ele deve capturar a exceção, registrar no log do agente no `SavingsDashboard` e fazer o passthrough exato e imediato da mensagem original. A execução e a autonomia do agente no OmniRift nunca podem ser bloqueadas por falhas na camada de compressão.

## Estratégia de testes (TDD)

1. **Testes Unitários:**
   - `AgentCompressionConfig`: Validação de desserialização e aplicação de perfis.
   - `CCRSandbox`: Teste de isolamento rígido (Agente A requisita `hash_X`, recebe `None` mesmo que `hash_X` exista no sandbox do Agente B).
2. **Testes de Integração:**
   - `RiftInterceptor`: Mock do ciclo do OmniRift. Verificar se o interceptor chama o pipeline do SP1 corretamente e repassa o resultado.
   - Fail-open: Injetar falha no SP1 e garantir que o agente recebe o contexto original sem interrupção.
3. **Testes de UI/UX (E2E):**
   - Iniciar dois agentes simultaneamente no canvas, comprimir contextos distintos. Verificar se o `SavingsDashboard` mostra as métricas separadas e corretas (incluindo o rótulo "Estimado" quando aplicável ao tokenizer).
   - Verificar se o botão "Recuperar Original" do Dashboard busca corretamente via `CCRSandbox`.

## Critérios de sucesso

- Usuário consegue ativar/desligar a compressão e ajustar o TTL para um agente específico no OmniRift sem reiniciar o canvas.
- A economia de tokens é exibida por agente de forma honesta e rotulada (exato vs. aproximado) no `SavingsDashboard`.
- O isolamento do CCR é garantido: a compressão/contexto do Agente A não é visível ou recuperável no escopo do Agente B.
- Latência adicionada ao ciclo de execução do agente pelo `RiftInterceptor` (excluindo rede externa) é inferior a 50ms.
- Ocorrência de erros no motor de compressão resulta em 100% de passthrough transparente, sem perda de fluxo do agente.

## Riscos & mitigações

| Riscos | Mitigações |
| :--- | :--- |
| Vazamento de estado/contexto entre agentes no canvas concorrente. | Uso obrigatório de `agent_id` como prefixo/namespace no `CCRStore` gerido pelo `CCRSandbox`. |
| Latência excessiva introduzida no ciclo síncrono do OmniRift. | Execução do processo de compressão de forma assíncrona; implementação de timeout com fail-open garantido (ex: 500ms). |
| Confusão do usuário sobre o valor real dos tokens poupados. | Rótulos explícitos no `SavingsDashboard` herdando o estado de medição (`exact` ou `estimated`) do SP1. |
| Estouro de armazenamento CCR local por agentes de longa duração. | Respeitar o TTL configurado no `AgentCompressionConfig`, executando a rotina de garbage collection do `CCRStore` sem virar perda silenciosa de dados ativos. |

## Próximos passos

1. Implementar o modelo de dados do `AgentCompressionConfig` e expô-lo nas configurações do nó do agente na UI do OmniRift.
2. Construir o `CCRSandbox` encapsulando o `CCRStore` com lógica de isolamento por `agent_id`.
3. Desenvolver o `RiftInterceptor` conectando-o à superfície do SP2 e validando o fluxo de fail-open.
4. Integrar o `SavingsDashboard` ao pipeline de eventos do OmniRift para renderização de métricas e recuperação de contexto.