# Spec — OmniCompression · SP2 Superfícies

> **Status:** draft v0 — gerado em paralelo (2026-06-18) e auditado por painel externo. Fixes técnicos do audit ainda pendentes (ver `_audit-sp2-sp5-2026-06-18.md`): SP2 precedência `per-call > env > arquivo`; SP3 falha de candidato contabilizada como falha (não passthrough); SP4 `LearnedCompressor` = impl do trait existente + CCR obrigatório antes de compressão abstractiva; SP5 caminho primário = SP1 in-process. **NÃO é spec final — aguardando review.**


> **Origem:** OmniForge / OmniCompress
> **Escopo:** Expor o motor SP1 via três superfícies de entrega finas (Lib Python, Proxy HTTP, MCP Server) sem reimplementar lógica de compressão.
> **Depende de:** SP1 (OmniCompress Core — `CompressionPipeline`, `CCRStore`, `CompressResult`)
> **Princípio:** Fronteiras limpas e fail-open. Cada superfície é apenas uma camada de orquestração e tradução de formato sobre o core determinístico.

## Escopo
**Dentro:**
- Criação da Lib pública Python com API estável (`compress(messages, **cfg) -> CompressResult`), contratos de versionamento (semver) e gerenciamento de configuração (env/arquivo/per-call).
- Desenvolvimento de um Proxy HTTP local compatível com as APIs de LLM de mercado (formatos OpenAI e Anthropic), atuando como drop-in para o cliente.
- Implementação de suporte a streaming (SSE) no Proxy, roteamento por provedor e injeção do tool de retrieve do CCR.
- Criação de um MCP Server expondo as tools `omnicompress_compress`, `omnicompress_retrieve` e `omnicompress_stats`.

**Fora:**
- Reimplementação de algoritmos de compressão, tokenizers ou regras de roteamento de conteúdo.
- Modificações no motor do SP1 (`CompressionPipeline`, `Compressor` trait, `CCRStore`).
- Infraestrutura de nuvem, autoescalabilidade ou gerenciamento de chaves de API de provedores.

**Não-objetivos explícitos:**
- O Proxy não fará cache agressivo de respostas de provedores, focando apenas na compressão de entrada e descompressão de saída.
- A Lib Python não gerenciará concorrência de chamadas assíncronas de LLM; sua responsabilidade limita-se à invocação do pipeline de compressão.

## Arquitetura — unidades de fronteira clara

| Unidade | Responsabilidade | Interface (resumo) | Depende de |
| :--- | :--- | :--- | :--- |
| **Python Lib** | Bindings PyO3 e API estável para uso direto programaticamente. | `compress(messages, **cfg) -> CompressResult` | SP1 (`CompressionPipeline`) |
| **HTTP Proxy** | Servidor intermediário drop-in. Intercepta requisições, comprime input, encaminha, retransmite SSE, injeta tool de retrieve. | Endpoints compatíveis com REST/SSE de mercado. | SP1 (`CompressionPipeline`, `CCRStore`) |
| **MCP Server** | Host MCP que oferece ferramentas de compressão e recuperação de contexto para agentes. | JSON-RPC Tools: `omnicompress_compress`, `omnicompress_retrieve`, `omnicompress_stats` | SP1 (`CompressionPipeline`, `CCRStore`) |

## Fluxo de dados

```text
[Cliente / Agente]
       │
       ├── (Lib) ──> compress(messages) ──> [SP1 CompressionPipeline] ──> CompressResult
       │
       ├── (Proxy) ──> Intercepta request ──> [SP1 CompressionPipeline.compress(messages)]
       │                  │                       │
       │                  │                       ├──> Mensagens comprimidas + CCR_refs
       │                  │                       │
       │                  └──> Encaminha para Provedor de LLM ──> Recebe Stream/Response
       │                              │
       │                              └──> Devolve ao Cliente (SSE pass-through ou JSON)
       │
       └── (MCP) ──> Tool Call `omnicompress_compress`
                            │
                            └──> [SP1 CompressionPipeline] ──> Retorna CCR_refs e mensagens comprimidas
```

## Tratamento de erros — fail-open
- **Lib Python:** Erros de binding ou de execução do motor são capturados e resultam em exceções tipadas, garantindo que o dado original nunca seja perdido silenciosamente.
- **Proxy HTTP:** Qualquer falha no `CompressionPipeline` ou em mapeamento de schema faz o Proxy entrar em modo *passthrough* absoluto, encaminhando a requisição original intocada ao provedor.
- **MCP Server:** Falhas na execução das tools retornam um erro padrão de JSON-RPC para o host MCP. Se a compressão falhar, a tool `omnicompress_compress` retorna as mensagens originais sem compressão.

## Estratégia de testes (TDD)
1. **Lib Python:** Testes de contrato de versionamento (semver) e resolução de configuração (env > arquivo > per-call).
2. **Proxy HTTP:** Mocks de provedores OpenAI e Anthropic. Testes de parsing de request, compressão de payload, e retransmissão de SSE (Server-Sent Events) preservando os limites de chunk.
3. **MCP Server:** Validação dos schemas JSON de entrada e saída das tools. Testes de simulação de host chamando `omnicompress_retrieve` com um hash válido e invalido retornado pelo `CCRStore`.

## Critérios de sucesso
- API Python exposta estável e versionada, permitindo upgrade do core sem quebrar clientes.
- Proxy HTTP funciona como drop-in com zero alterações no código do cliente para OpenAI/Anthropic, incluindo streaming SSE perfeito.
- MCP Server interoperável com hosts MCP padrão, conseguindo comprimir payload, recuperar por hash e retornar estatísticas.
- Toda falha na camada de superfície cai gracefully em passthrough, sem perda de dados.

## Riscos & mitigações

| Risco | Mitigação |
| :--- | :--- |
| Mudanças não documentadas no schema das APIs de LLM de mercado quebrando o Proxy. | Manter roteadores isolados por provedor e aplicar fail-open estrito, garantindo que o passthrough ocorra antes do parsing se a estrutura base divergir. |
| Sobrecarga de latência no Proxy ao processar SSE (streaming). | Implementar pass-through de bytes direto para respostas não estruturadas e parsing otimizado para injeção de tool apenas quando necessário. |
| Incompatibilidade de schemas no protocolo MCP. | Adesão estrita ao JSON-RPC e schemas de tool documentados, com testes de integração simulando agentes host. |

## Próximos passos
1. Implementar e congelar a interface pública da Lib Python.
2. Construir o roteador de provedores e o pipeline de SSE do Proxy HTTP.
3. Desenvolver o MCP Server e testar a injeção de tools em um agente host nativo.