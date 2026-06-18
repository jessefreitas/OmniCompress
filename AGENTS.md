# AGENTS.md — OmniCompress

Guia para agentes (e humanos) que trabalham neste repositório.

## Antes de qualquer coisa

Leia o spec ativo: [`docs/specs/2026-06-18-omnicompression-sp1-core-design.md`](docs/specs/2026-06-18-omnicompression-sp1-core-design.md).
Ele define escopo, arquitetura (unidades), CCR, cross-platform e critérios de sucesso do SP1.

## Regras de desenvolvimento

- **Dev flow:** worktree isolado → TDD → quality gate → PR. Sem card no Kanban, sem código.
- **Clean-room:** implementação 100% própria e original.
- **Fail-open sempre:** qualquer erro de compressão → passthrough do bloco original. Nunca perde conteúdo.
- **Honestidade de métrica:** tokenizer exato onde há (OpenAI/Ollama); Anthropic = `~estimado` rotulado.

## Estrutura (planejada — SP1)

```
crates/omnicompress-core/   # motor Rust (router, compressores, CCR, trait CCRStore)
crates/omnicompress-py/     # binding Python (PyO3)
docs/specs/                 # specs por sub-projeto
```

## Decisões travadas (brainstorming 2026-06-18)

Produto novo · wedge = tech-superior (modelo PT-BR) + nativo no OmniRift · clean-room ·
Rust+Python · CCR default = `redb` embarcado (cross-platform; Redis opt-in).
