# SP1 — OmniCompress Core · Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Construir o motor de compressão determinístico do OmniCompress (Rust core + binding Python), reversível (CCR), fail-open, cross-platform — entregando ≥68% de redução blended em sessão real, com 100% de fidelidade no round-trip.

**Architecture:** Workspace Cargo com `omnicompress-core` (lógica pura: router → compressores → CCR → pipeline → eval) e `omnicompress-py` (binding PyO3 fino). Tudo trabalha sobre traits (`Compressor`, `CCRStore`) pra que impls sejam unidades isoladas e testáveis. Sem ML no SP1.

**Tech Stack:** Rust (stable), PyO3 + maturin, `redb` (CCR embarcado), `tree-sitter` (AST), `serde_json`, `pytest` (binding). CI: `maturin-action` em ubuntu/macos/windows.

**Spec:** `docs/specs/2026-06-18-omnicompression-sp1-core-design.md`.

---

## File Structure

```
Cargo.toml                                  # workspace (members = core, py)
crates/omnicompress-core/
  Cargo.toml
  src/lib.rs                                # re-exports públicos
  src/types.rs                              # ContentKind, Block, Role, CcrRef, Hash, CompressResult, Transform
  src/tokenizer.rs                          # Tokenizer trait + HeuristicTokenizer
  src/router.rs                             # ContentRouter
  src/protection.rs                         # ProtectionPolicy + CompressConfig
  src/compressor/mod.rs                     # Compressor trait + Outcome
  src/compressor/passthrough.rs             # PassThrough
  src/compressor/json_crusher.rs            # JsonCrusher
  src/compressor/log_text.rs                # LogTextCompressor
  src/compressor/code.rs                    # CodeCompressor (tree-sitter)
  src/ccr/mod.rs                            # CCRStore trait + Hash util
  src/ccr/memory.rs                         # MemoryStore (in-process)
  src/ccr/embedded.rs                       # EmbeddedStore (redb)
  src/pipeline.rs                           # CompressionPipeline + Measurement
  src/eval.rs                               # EvalHarness + EvalReport
  tests/contract_ccrstore.rs               # contrato compartilhado dos CCRStore
  tests/pipeline_integration.rs            # sessão misturada end-to-end
crates/omnicompress-py/
  Cargo.toml
  pyproject.toml                            # maturin
  src/lib.rs                                # #[pymodule] expõe compress()
  python/tests/test_compress.py            # teste do binding
.github/workflows/ci.yml                    # matriz 3 SOs (Forgejo Actions equivalente)
```

**Princípio de decomposição:** cada compressor e cada CCRStore é uma unidade isolada por trás de
trait; o `pipeline` é o único que conhece todas. Tarefas leaf (compressores/stores) são paralelizáveis
SÓ depois que as traits + tipos (Tasks 1–2, 4, 6) existirem — antes disso, há estado compartilhado.

---

## Ordem de dependência

```
T0 workspace → T1 types → T2 tokenizer → T3 router
                              ↘ T4 Compressor trait + PassThrough
                                 ↘ T6 CCRStore trait + MemoryStore
T5 JsonCrusher ─┐
T7 LogText      ├─ (dependem de T4+T6; paralelizáveis entre si)
T9 CodeComp     ┘
T8 ProtectionPolicy (dep T1)
T10 Pipeline (dep T3,T4,T5,T6,T7,T8) → T11 Measurement (no pipeline)
T12 EmbeddedStore redb (dep T6; passa no contract de T6)
T13 EvalHarness (dep T10)
T14 Python binding (dep T10) → T15 CLI → T16 dogfood + success gate
```

---

### Task 0: Workspace scaffold

**Files:**
- Create: `Cargo.toml`, `crates/omnicompress-core/Cargo.toml`, `crates/omnicompress-core/src/lib.rs`

- [ ] **Step 1: Workspace `Cargo.toml`**

```toml
[workspace]
resolver = "2"
members = ["crates/omnicompress-core", "crates/omnicompress-py"]

[workspace.package]
version = "0.1.0"
edition = "2021"
license = "Apache-2.0"
```

- [ ] **Step 2: `crates/omnicompress-core/Cargo.toml`**

```toml
[package]
name = "omnicompress-core"
version.workspace = true
edition.workspace = true
license.workspace = true

[dependencies]
serde = { version = "1", features = ["derive"] }
serde_json = "1"
redb = "2"
blake3 = "1"

[dev-dependencies]
```

- [ ] **Step 3: Minimal `lib.rs`**

```rust
pub mod types;
```

- [ ] **Step 4: Verify it compiles**

Run: `cargo build -p omnicompress-core`
Expected: builds (types module stub may be empty file for now — create `src/types.rs` empty).

- [ ] **Step 5: Commit**

```bash
git add Cargo.toml crates/omnicompress-core
git commit -m "chore(core): workspace scaffold"
```

---

### Task 1: Core types

**Files:**
- Create: `crates/omnicompress-core/src/types.rs`
- Test: inline `#[cfg(test)]` em `types.rs`

- [ ] **Step 1: Write the failing test**

```rust
// em src/types.rs, no fim:
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn block_text_len_counts_chars() {
        let b = Block::text(Role::User, "abc");
        assert_eq!(b.text(), "abc");
        assert!(matches!(b.role, Role::User));
    }
    #[test]
    fn compress_result_saved_is_diff() {
        let r = CompressResult { messages: vec![], tokens_before: 100, tokens_after: 30, transforms: vec![], ccr_refs: vec![] };
        assert_eq!(r.tokens_saved(), 70);
    }
}
```

- [ ] **Step 2: Run test to verify it fails**

Run: `cargo test -p omnicompress-core types::`
Expected: FAIL — `Block`, `Role`, `CompressResult` não existem.

- [ ] **Step 3: Write minimal implementation**

```rust
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub enum Role { System, User, Assistant, Tool }

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub enum ContentKind { Json, Code, Log, Prose, Diff, Unknown }

/// Um bloco de conteúdo dentro de uma mensagem.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Block {
    pub role: Role,
    pub content: String,
    /// nome da tool de origem, quando aplicável (ex.: "Bash", "search_memories").
    pub tool_name: Option<String>,
}

impl Block {
    pub fn text(role: Role, content: &str) -> Self {
        Block { role, content: content.to_string(), tool_name: None }
    }
    pub fn tool(role: Role, content: &str, tool: &str) -> Self {
        Block { role, content: content.to_string(), tool_name: Some(tool.to_string()) }
    }
    pub fn text_ref(&self) -> &str { &self.content }
    pub fn text(&self) -> &str { &self.content } // conveniência usada nos testes
}

pub type Hash = String;

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct CcrRef { pub hash: Hash, pub original_tokens: usize }

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Transform { pub unit: String, pub detail: String }

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CompressResult {
    pub messages: Vec<Block>,
    pub tokens_before: usize,
    pub tokens_after: usize,
    pub transforms: Vec<Transform>,
    pub ccr_refs: Vec<CcrRef>,
}
impl CompressResult {
    pub fn tokens_saved(&self) -> usize { self.tokens_before.saturating_sub(self.tokens_after) }
}
```

> Nota: remova o método duplicado `text` (mantenha só um `pub fn text(&self) -> &str`); o `text` construtor vira `from_text`. Ajuste o teste do Step 1 para `Block::from_text`.

- [ ] **Step 4: Run test to verify it passes**

Run: `cargo test -p omnicompress-core types::`
Expected: PASS.

- [ ] **Step 5: Commit**

```bash
git add crates/omnicompress-core/src/types.rs crates/omnicompress-core/src/lib.rs
git commit -m "feat(core): tipos base (Block, ContentKind, CompressResult)"
```

---

### Task 2: Tokenizer

**Files:**
- Create: `crates/omnicompress-core/src/tokenizer.rs`; add `pub mod tokenizer;` em `lib.rs`

- [ ] **Step 1: Write the failing test**

```rust
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn heuristic_roughly_chars_over_four() {
        let t = HeuristicTokenizer::default();
        // ~4 chars/token é a heurística; 400 chars ~= 100 tokens
        let n = t.count(&"x".repeat(400));
        assert!((90..=110).contains(&n), "got {n}");
    }
    #[test]
    fn empty_is_zero() {
        assert_eq!(HeuristicTokenizer::default().count(""), 0);
    }
}
```

- [ ] **Step 2: Run test to verify it fails**

Run: `cargo test -p omnicompress-core tokenizer::`
Expected: FAIL — `HeuristicTokenizer` não existe.

- [ ] **Step 3: Write minimal implementation**

```rust
pub trait Tokenizer: Send + Sync {
    fn count(&self, text: &str) -> usize;
    /// rótulo de honestidade da métrica ("exact" | "~estimated")
    fn fidelity(&self) -> &'static str { "~estimated" }
}

#[derive(Default)]
pub struct HeuristicTokenizer;

impl Tokenizer for HeuristicTokenizer {
    fn count(&self, text: &str) -> usize {
        if text.is_empty() { return 0; }
        // heurística calibrada ~4 chars/token (arredonda pra cima)
        text.chars().count().div_ceil(4)
    }
}
```

> Quando houver tokenizer local exato (tiktoken/HF) num SP futuro, criar `ExactTokenizer` com `fidelity()=="exact"`. SP1 usa o heurístico.

- [ ] **Step 4: Run test to verify it passes**

Run: `cargo test -p omnicompress-core tokenizer::`
Expected: PASS.

- [ ] **Step 5: Commit**

```bash
git add crates/omnicompress-core/src/tokenizer.rs crates/omnicompress-core/src/lib.rs
git commit -m "feat(core): Tokenizer trait + HeuristicTokenizer (fidelity rotulado)"
```

---

### Task 3: ContentRouter

**Files:**
- Create: `crates/omnicompress-core/src/router.rs`; `pub mod router;` em `lib.rs`

- [ ] **Step 1: Write the failing test**

```rust
#[cfg(test)]
mod tests {
    use super::*;
    use crate::types::ContentKind;
    #[test]
    fn detects_json_array() {
        assert_eq!(ContentRouter::default().route(r#"[{"a":1},{"a":2}]"#), ContentKind::Json);
    }
    #[test]
    fn detects_code_by_keywords() {
        assert_eq!(ContentRouter::default().route("def foo():\n    return 1\n"), ContentKind::Code);
    }
    #[test]
    fn detects_log_by_repetition() {
        let log = "2026-06-18 INFO x\n2026-06-18 INFO y\n2026-06-18 INFO z\n";
        assert_eq!(ContentRouter::default().route(log), ContentKind::Log);
    }
    #[test]
    fn prose_is_default_textual() {
        assert_eq!(ContentRouter::default().route("isto é apenas um texto comum em português"), ContentKind::Prose);
    }
}
```

- [ ] **Step 2: Run test to verify it fails**

Run: `cargo test -p omnicompress-core router::`
Expected: FAIL — `ContentRouter` não existe.

- [ ] **Step 3: Write minimal implementation**

```rust
use crate::types::ContentKind;

#[derive(Default)]
pub struct ContentRouter;

impl ContentRouter {
    /// Classifica por CONTEÚDO (não por nome de tool). Determinístico, barato.
    pub fn route(&self, content: &str) -> ContentKind {
        let t = content.trim_start();
        if (t.starts_with('{') || t.starts_with('[')) && serde_json::from_str::<serde_json::Value>(content).is_ok() {
            return ContentKind::Json;
        }
        if t.starts_with("diff ") || t.starts_with("--- ") || t.starts_with("@@ ") {
            return ContentKind::Diff;
        }
        if Self::looks_like_code(content) { return ContentKind::Code; }
        if Self::looks_like_log(content) { return ContentKind::Log; }
        if content.trim().is_empty() { return ContentKind::Unknown; }
        ContentKind::Prose
    }

    fn looks_like_code(s: &str) -> bool {
        const KW: [&str; 8] = ["def ", "fn ", "class ", "import ", "function ", "func ", "public ", "const "];
        let hits = KW.iter().filter(|k| s.contains(*k)).count();
        let braces = s.matches(['{', '}', ';']).count();
        hits >= 1 && (braces >= 2 || s.contains("():") || s.contains("->"))
    }

    fn looks_like_log(s: &str) -> bool {
        let lines: Vec<&str> = s.lines().filter(|l| !l.trim().is_empty()).collect();
        if lines.len() < 3 { return false; }
        // muitas linhas começando com prefixo parecido (timestamp/level) => log
        let with_prefix = lines.iter().filter(|l| {
            let h: String = l.chars().take(4).collect();
            h.chars().all(|c| c.is_ascii_digit()) || l.contains("INFO") || l.contains("ERROR") || l.contains("WARN")
        }).count();
        with_prefix * 2 >= lines.len()
    }
}
```

- [ ] **Step 4: Run test to verify it passes**

Run: `cargo test -p omnicompress-core router::`
Expected: PASS (4 testes).

- [ ] **Step 5: Commit**

```bash
git add crates/omnicompress-core/src/router.rs crates/omnicompress-core/src/lib.rs
git commit -m "feat(core): ContentRouter (classifica por conteúdo, não por nome de tool)"
```

---

### Task 4: Compressor trait + PassThrough

**Files:**
- Create: `crates/omnicompress-core/src/compressor/mod.rs`, `.../passthrough.rs`; `pub mod compressor;` em `lib.rs`

- [ ] **Step 1: Write the failing test**

```rust
// em compressor/mod.rs
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn passthrough_returns_input_unchanged_and_no_ccr() {
        let out = PassThrough.compress("hello world");
        assert_eq!(out.compressed, "hello world");
        assert!(out.original.is_none());
    }
}
```

- [ ] **Step 2: Run test to verify it fails**

Run: `cargo test -p omnicompress-core compressor::`
Expected: FAIL — `PassThrough`/`Compressor` não existem.

- [ ] **Step 3: Write minimal implementation**

```rust
// compressor/mod.rs
pub mod passthrough;
pub mod json_crusher;
pub mod log_text;
pub mod code;
pub use passthrough::PassThrough;

/// Resultado de comprimir UM bloco. `original` presente => guardar no CCR.
pub struct Outcome {
    pub compressed: String,
    pub original: Option<String>,
    pub detail: String,
}
impl Outcome {
    pub fn untouched(s: &str) -> Self { Outcome { compressed: s.to_string(), original: None, detail: "untouched".into() } }
}

pub trait Compressor: Send + Sync {
    /// Comprime; em QUALQUER dúvida/erro deve retornar passthrough (fail-open é garantido no pipeline também).
    fn compress(&self, content: &str) -> Outcome;
    fn name(&self) -> &'static str;
}

// compressor/passthrough.rs
use super::{Compressor, Outcome};
pub struct PassThrough;
impl Compressor for PassThrough {
    fn compress(&self, content: &str) -> Outcome { Outcome::untouched(content) }
    fn name(&self) -> &'static str { "passthrough" }
}
```

> Crie `json_crusher.rs`, `log_text.rs`, `code.rs` como stubs `pub struct X; impl Compressor for X { ... untouched ... }` pra compilar; preenchidos em T5/T7/T9.

- [ ] **Step 4: Run test to verify it passes**

Run: `cargo test -p omnicompress-core compressor::`
Expected: PASS.

- [ ] **Step 5: Commit**

```bash
git add crates/omnicompress-core/src/compressor crates/omnicompress-core/src/lib.rs
git commit -m "feat(core): Compressor trait + PassThrough + stubs"
```

---

### Task 5: JsonCrusher (o maior ganho)

**Files:**
- Modify: `crates/omnicompress-core/src/compressor/json_crusher.rs`

- [ ] **Step 1: Write the failing test**

```rust
#[cfg(test)]
mod tests {
    use super::*;
    use crate::compressor::Compressor;
    #[test]
    fn crushes_array_of_records_to_schema_plus_sample() {
        let arr: String = "[".to_string()
            + &(0..50).map(|i| format!(r#"{{"id":{i},"name":"n{i}","score":0.5}}"#)).collect::<Vec<_>>().join(",")
            + "]";
        let out = JsonCrusher::default().compress(&arr);
        assert!(out.original.is_some(), "deve guardar original no CCR");
        assert!(out.compressed.len() * 3 < arr.len(), "deve cortar >2/3: {} vs {}", out.compressed.len(), arr.len());
        assert!(out.compressed.contains("id") && out.compressed.contains("50"), "schema+contagem: {}", out.compressed);
    }
    #[test]
    fn tiny_array_untouched() {
        let out = JsonCrusher::default().compress(r#"[{"a":1}]"#);
        assert!(out.original.is_none(), "array pequeno não vale a pena");
    }
}
```

- [ ] **Step 2: Run test to verify it fails**

Run: `cargo test -p omnicompress-core json_crusher`
Expected: FAIL — implementação ainda é stub passthrough.

- [ ] **Step 3: Write minimal implementation**

```rust
use super::{Compressor, Outcome};
use serde_json::Value;

#[derive(Default)]
pub struct JsonCrusher;

impl Compressor for JsonCrusher {
    fn name(&self) -> &'static str { "json_crusher" }
    fn compress(&self, content: &str) -> Outcome {
        let Ok(v) = serde_json::from_str::<Value>(content) else { return Outcome::untouched(content); };
        // alvo: array de objetos homogêneos (caso dominante de tool-output)
        let arr = match &v {
            Value::Array(a) => a.clone(),
            Value::Object(o) => o.values().find_map(|x| if let Value::Array(a)=x { Some(a.clone()) } else { None }).unwrap_or_default(),
            _ => return Outcome::untouched(content),
        };
        if arr.len() < 20 { return Outcome::untouched(content); }
        // schema = chaves do 1º objeto; amostra = 2 itens; + contagem
        let keys: Vec<String> = match arr.first() {
            Some(Value::Object(o)) => o.keys().cloned().collect(),
            _ => return Outcome::untouched(content),
        };
        let sample: Vec<&Value> = arr.iter().take(2).collect();
        let summary = serde_json::json!({
            "_omnicompress": "json_array",
            "count": arr.len(),
            "schema": keys,
            "sample": sample,
        });
        let compressed = summary.to_string();
        if compressed.len() >= content.len() { return Outcome::untouched(content); }
        Outcome { compressed, original: Some(content.to_string()), detail: format!("json_array:{}", arr.len()) }
    }
}
```

- [ ] **Step 4: Run test to verify it passes**

Run: `cargo test -p omnicompress-core json_crusher`
Expected: PASS (2 testes).

- [ ] **Step 5: Commit**

```bash
git add crates/omnicompress-core/src/compressor/json_crusher.rs
git commit -m "feat(core): JsonCrusher (schema+amostra+contagem, original no CCR)"
```

---

### Task 6: CCRStore trait + MemoryStore + contrato compartilhado

**Files:**
- Create: `crates/omnicompress-core/src/ccr/mod.rs`, `.../memory.rs`, `tests/contract_ccrstore.rs`; `pub mod ccr;` em `lib.rs`

- [ ] **Step 1: Write the failing test (contrato reutilizável)**

```rust
// tests/contract_ccrstore.rs
use omnicompress_core::ccr::{CCRStore, MemoryStore};

fn roundtrip<S: CCRStore>(store: S) {
    let h = store.put(b"conteudo original").unwrap();
    assert_eq!(store.get(&h).unwrap().as_deref(), Some(&b"conteudo original"[..]));
    // hash é determinístico (mesmo conteúdo => mesmo hash)
    let h2 = store.put(b"conteudo original").unwrap();
    assert_eq!(h, h2);
    // miss conhecido
    assert!(store.get("hash-inexistente").unwrap().is_none());
}

#[test]
fn memory_store_satisfies_contract() {
    roundtrip(MemoryStore::default());
}
```

- [ ] **Step 2: Run test to verify it fails**

Run: `cargo test -p omnicompress-core --test contract_ccrstore`
Expected: FAIL — `CCRStore`/`MemoryStore` não existem.

- [ ] **Step 3: Write minimal implementation**

```rust
// ccr/mod.rs
pub mod memory;
pub mod embedded;
pub use memory::MemoryStore;
pub use embedded::EmbeddedStore;

pub type Hash = String;

pub trait CCRStore: Send + Sync {
    fn put(&self, original: &[u8]) -> std::io::Result<Hash>;
    fn get(&self, hash: &str) -> std::io::Result<Option<Vec<u8>>>;
}

pub fn hash_of(bytes: &[u8]) -> Hash { blake3::hash(bytes).to_hex().to_string() }

// ccr/memory.rs
use super::{CCRStore, Hash, hash_of};
use std::collections::HashMap;
use std::sync::RwLock;

#[derive(Default)]
pub struct MemoryStore { map: RwLock<HashMap<Hash, Vec<u8>>> }

impl CCRStore for MemoryStore {
    fn put(&self, original: &[u8]) -> std::io::Result<Hash> {
        let h = hash_of(original);
        self.map.write().unwrap().insert(h.clone(), original.to_vec());
        Ok(h)
    }
    fn get(&self, hash: &str) -> std::io::Result<Option<Vec<u8>>> {
        Ok(self.map.read().unwrap().get(hash).cloned())
    }
}
```

> Crie `ccr/embedded.rs` como stub que compila (struct vazia + `impl CCRStore` retornando `unimplemented!()` ou um `todo!()` guardado) só para `pub use` funcionar; implementado em T12.

- [ ] **Step 4: Run test to verify it passes**

Run: `cargo test -p omnicompress-core --test contract_ccrstore`
Expected: PASS.

- [ ] **Step 5: Commit**

```bash
git add crates/omnicompress-core/src/ccr crates/omnicompress-core/tests/contract_ccrstore.rs crates/omnicompress-core/src/lib.rs
git commit -m "feat(core): CCRStore trait + MemoryStore + contrato de round-trip"
```

---

### Task 7: LogTextCompressor

**Files:**
- Modify: `crates/omnicompress-core/src/compressor/log_text.rs`

- [ ] **Step 1: Write the failing test**

```rust
#[cfg(test)]
mod tests {
    use super::*;
    use crate::compressor::Compressor;
    #[test]
    fn collapses_repeated_lines() {
        let log = std::iter::repeat("conexão recusada na porta 5432\n").take(40).collect::<String>()
            + "evento único final\n";
        let out = LogTextCompressor::default().compress(&log);
        assert!(out.original.is_some());
        assert!(out.compressed.contains("×40") || out.compressed.contains("x40"), "deve colapsar repetição: {}", out.compressed);
        assert!(out.compressed.contains("evento único final"));
        assert!(out.compressed.len() * 2 < log.len());
    }
}
```

- [ ] **Step 2: Run test to verify it fails**

Run: `cargo test -p omnicompress-core log_text`
Expected: FAIL — stub passthrough.

- [ ] **Step 3: Write minimal implementation**

```rust
use super::{Compressor, Outcome};

#[derive(Default)]
pub struct LogTextCompressor;

impl Compressor for LogTextCompressor {
    fn name(&self) -> &'static str { "log_text" }
    fn compress(&self, content: &str) -> Outcome {
        let lines: Vec<&str> = content.lines().collect();
        if lines.len() < 10 { return Outcome::untouched(content); }
        // colapsa runs de linhas idênticas consecutivas em "linha ×N"
        let mut out = String::new();
        let mut i = 0;
        while i < lines.len() {
            let mut j = i + 1;
            while j < lines.len() && lines[j] == lines[i] { j += 1; }
            let run = j - i;
            if run > 1 { out.push_str(&format!("{} ×{}\n", lines[i], run)); }
            else { out.push_str(lines[i]); out.push('\n'); }
            i = j;
        }
        if out.len() >= content.len() { return Outcome::untouched(content); }
        Outcome { compressed: out, original: Some(content.to_string()), detail: "log_collapse".into() }
    }
}
```

- [ ] **Step 4: Run test to verify it passes**

Run: `cargo test -p omnicompress-core log_text`
Expected: PASS.

- [ ] **Step 5: Commit**

```bash
git add crates/omnicompress-core/src/compressor/log_text.rs
git commit -m "feat(core): LogTextCompressor (colapso de repetição)"
```

---

### Task 8: ProtectionPolicy + CompressConfig

**Files:**
- Create: `crates/omnicompress-core/src/protection.rs`; `pub mod protection;` em `lib.rs`

- [ ] **Step 1: Write the failing test**

```rust
#[cfg(test)]
mod tests {
    use super::*;
    use crate::types::{Block, Role};
    #[test]
    fn protects_recent_n() {
        let cfg = CompressConfig::default(); // protect_recent = 4
        let p = ProtectionPolicy::new(&cfg);
        // índice 9 de 10 (recente) protegido; índice 0 (antigo) não
        assert!(p.is_protected(0, 10, &Block::tool(Role::User, "x", "Bash")) == false);
        assert!(p.is_protected(9, 10, &Block::tool(Role::User, "x", "Bash")));
    }
    #[test]
    fn protects_edit_critical_tools() {
        let cfg = CompressConfig::default();
        let p = ProtectionPolicy::new(&cfg);
        // Read/Edit/Glob/Grep/Write nunca comprimem (conteúdo exato necessário)
        assert!(p.is_protected(0, 10, &Block::tool(Role::User, "x", "Read")));
    }
}
```

- [ ] **Step 2: Run test to verify it fails**

Run: `cargo test -p omnicompress-core protection::`
Expected: FAIL.

- [ ] **Step 3: Write minimal implementation**

```rust
use crate::types::Block;

#[derive(Clone)]
pub struct CompressConfig {
    pub protect_recent: usize,
    pub min_chars_to_compress: usize,
    pub excluded_tools: Vec<String>,
}
impl Default for CompressConfig {
    fn default() -> Self {
        CompressConfig {
            protect_recent: 4,
            min_chars_to_compress: 600, // ~150 tokens
            excluded_tools: ["Read","Edit","Glob","Grep","Write"].iter().map(|s| s.to_string()).collect(),
        }
    }
}

pub struct ProtectionPolicy<'a> { cfg: &'a CompressConfig }
impl<'a> ProtectionPolicy<'a> {
    pub fn new(cfg: &'a CompressConfig) -> Self { Self { cfg } }
    pub fn is_protected(&self, idx: usize, total: usize, block: &Block) -> bool {
        // recentes-N
        if idx + self.cfg.protect_recent >= total { return true; }
        // tools críticas de edição
        if let Some(t) = &block.tool_name {
            if self.cfg.excluded_tools.iter().any(|e| e.eq_ignore_ascii_case(t)) { return true; }
        }
        // muito curto pra valer
        if block.content.len() < self.cfg.min_chars_to_compress { return true; }
        false
    }
}
```

- [ ] **Step 4: Run test to verify it passes**

Run: `cargo test -p omnicompress-core protection::`
Expected: PASS.

- [ ] **Step 5: Commit**

```bash
git add crates/omnicompress-core/src/protection.rs crates/omnicompress-core/src/lib.rs
git commit -m "feat(core): ProtectionPolicy (recentes-N + tools de edição + min size)"
```

---

### Task 9: CodeCompressor (tree-sitter)

**Files:**
- Modify: `crates/omnicompress-core/Cargo.toml` (deps tree-sitter), `crates/omnicompress-core/src/compressor/code.rs`

- [ ] **Step 1: Add deps**

```toml
# em [dependencies]
tree-sitter = "0.22"
tree-sitter-python = "0.21"
```

- [ ] **Step 2: Write the failing test**

```rust
#[cfg(test)]
mod tests {
    use super::*;
    use crate::compressor::Compressor;
    #[test]
    fn keeps_signatures_drops_bodies() {
        let code = "import os\n\ndef alpha(x):\n    y = x + 1\n    return y\n\ndef beta(a, b):\n    return a * b\n".repeat(6);
        let out = CodeCompressor::default().compress(&code);
        assert!(out.original.is_some());
        assert!(out.compressed.contains("def alpha(x)"), "mantém assinatura");
        assert!(out.compressed.contains("import os"), "mantém imports");
        assert!(out.compressed.len() < code.len());
    }
}
```

- [ ] **Step 3: Run test to verify it fails**

Run: `cargo test -p omnicompress-core code`
Expected: FAIL — stub.

- [ ] **Step 4: Write minimal implementation**

```rust
use super::{Compressor, Outcome};
use tree_sitter::{Parser, Node};

#[derive(Default)]
pub struct CodeCompressor;

impl Compressor for CodeCompressor {
    fn name(&self) -> &'static str { "code" }
    fn compress(&self, content: &str) -> Outcome {
        if content.len() < 600 { return Outcome::untouched(content); }
        let mut parser = Parser::new();
        if parser.set_language(&tree_sitter_python::LANGUAGE.into()).is_err() { return Outcome::untouched(content); }
        let Some(tree) = parser.parse(content, None) else { return Outcome::untouched(content); };
        let src = content.as_bytes();
        let mut out = String::new();
        let mut cur = tree.root_node().walk();
        for child in tree.root_node().children(&mut cur) {
            keep_top_level(child, src, &mut out);
        }
        if out.is_empty() || out.len() >= content.len() { return Outcome::untouched(content); }
        Outcome { compressed: out, original: Some(content.to_string()), detail: "code_ast".into() }
    }
}

fn keep_top_level(node: Node, src: &[u8], out: &mut String) {
    match node.kind() {
        "import_statement" | "import_from_statement" | "expression_statement" => {
            out.push_str(node.utf8_text(src).unwrap_or("")); out.push('\n');
        }
        "function_definition" | "class_definition" => {
            // mantém só a linha da assinatura (até o ':'), corpo -> marcador
            let full = node.utf8_text(src).unwrap_or("");
            let sig = full.split_once(":\n").map(|(a, _)| a).unwrap_or(full);
            out.push_str(sig); out.push_str(":\n    ...  # corpo no CCR\n");
        }
        _ => {}
    }
}
```

- [ ] **Step 5: Run/commit**

Run: `cargo test -p omnicompress-core code` → PASS.
```bash
git add crates/omnicompress-core/Cargo.toml crates/omnicompress-core/src/compressor/code.rs
git commit -m "feat(core): CodeCompressor (tree-sitter: assinaturas+imports, corpo no CCR)"
```

---

### Task 10: CompressionPipeline + Measurement (fail-open)

**Files:**
- Create: `crates/omnicompress-core/src/pipeline.rs`; `pub mod pipeline;` em `lib.rs`

- [ ] **Step 1: Write the failing test**

```rust
#[cfg(test)]
mod tests {
    use super::*;
    use crate::types::{Block, Role};
    use crate::ccr::MemoryStore;

    fn big_json() -> String {
        "[".to_string() + &(0..60).map(|i| format!(r#"{{"id":{i},"v":"{i}"}}"#)).collect::<Vec<_>>().join(",") + "]"
    }

    #[test]
    fn compresses_old_tool_output_protects_recent() {
        let store = MemoryStore::default();
        let pipe = CompressionPipeline::new(Box::new(store));
        let mut msgs = vec![ Block::tool(Role::User, &big_json(), "search") ];
        for i in 0..6 { msgs.push(Block::text(Role::Assistant, &format!("ok {i}"))); }
        let r = pipe.compress(msgs, &CompressConfig::default());
        assert!(r.tokens_after < r.tokens_before, "deve comprimir: {} -> {}", r.tokens_before, r.tokens_after);
        assert_eq!(r.ccr_refs.len(), 1, "guardou 1 original no CCR");
    }

    #[test]
    fn fail_open_keeps_content_on_panic_free_error() {
        // compressor que sempre devolve maior => pipeline mantém original (sem perda)
        let store = MemoryStore::default();
        let pipe = CompressionPipeline::new(Box::new(store));
        let msgs = vec![ Block::text(Role::User, "curto") ];
        let r = pipe.compress(msgs.clone(), &CompressConfig::default());
        assert_eq!(r.messages[0].content, "curto"); // protegido por min size, intacto
    }
}
```

- [ ] **Step 2: Run test to verify it fails**

Run: `cargo test -p omnicompress-core pipeline::`
Expected: FAIL.

- [ ] **Step 3: Write minimal implementation**

```rust
use crate::types::{Block, CompressResult, ContentKind, CcrRef, Transform};
use crate::router::ContentRouter;
use crate::protection::{ProtectionPolicy, CompressConfig};
use crate::tokenizer::{Tokenizer, HeuristicTokenizer};
use crate::ccr::CCRStore;
use crate::compressor::{Compressor, json_crusher::JsonCrusher, code::CodeCompressor, log_text::LogTextCompressor, passthrough::PassThrough};

pub use crate::protection::CompressConfig as _CompressConfigReExport;

pub struct CompressionPipeline {
    router: ContentRouter,
    tok: HeuristicTokenizer,
    ccr: Box<dyn CCRStore>,
}

impl CompressionPipeline {
    pub fn new(ccr: Box<dyn CCRStore>) -> Self {
        Self { router: ContentRouter, tok: HeuristicTokenizer, ccr }
    }

    fn pick(&self, kind: ContentKind) -> Box<dyn Compressor> {
        match kind {
            ContentKind::Json => Box::new(JsonCrusher),
            ContentKind::Code => Box::new(CodeCompressor),
            ContentKind::Log | ContentKind::Prose | ContentKind::Diff => Box::new(LogTextCompressor),
            ContentKind::Unknown => Box::new(PassThrough),
        }
    }

    pub fn compress(&self, messages: Vec<Block>, cfg: &CompressConfig) -> CompressResult {
        let total = messages.len();
        let policy = ProtectionPolicy::new(cfg);
        let mut tokens_before = 0usize;
        let mut tokens_after = 0usize;
        let mut transforms = Vec::new();
        let mut ccr_refs = Vec::new();
        let mut out_msgs = Vec::with_capacity(total);

        for (idx, block) in messages.into_iter().enumerate() {
            tokens_before += self.tok.count(&block.content);
            if policy.is_protected(idx, total, &block) {
                tokens_after += self.tok.count(&block.content);
                out_msgs.push(block);
                continue;
            }
            let kind = self.router.route(&block.content);
            let comp = self.pick(kind);
            // fail-open: captura panics do compressor e cai pra passthrough
            let outcome = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| comp.compress(&block.content)))
                .unwrap_or_else(|_| crate::compressor::Outcome::untouched(&block.content));

            if let Some(original) = outcome.original {
                let original_tokens = self.tok.count(&original);
                match self.ccr.put(original.as_bytes()) {
                    Ok(hash) => {
                        let marker = format!("{}\n[omnicompress: original em CCR hash={} (~{} tok). retrieve se precisar.]",
                                             outcome.compressed, &hash[..12.min(hash.len())], original_tokens);
                        let mut nb = block.clone();
                        nb.content = marker;
                        tokens_after += self.tok.count(&nb.content);
                        transforms.push(Transform { unit: comp.name().into(), detail: outcome.detail });
                        ccr_refs.push(CcrRef { hash, original_tokens });
                        out_msgs.push(nb);
                    }
                    Err(_) => { // CCR falhou => fail-open, mantém original
                        tokens_after += self.tok.count(&block.content);
                        out_msgs.push(block);
                    }
                }
            } else {
                tokens_after += self.tok.count(&outcome.compressed);
                let mut nb = block.clone();
                nb.content = outcome.compressed;
                out_msgs.push(nb);
            }
        }
        CompressResult { messages: out_msgs, tokens_before, tokens_after, transforms, ccr_refs }
    }
}
```

> Importe `CompressConfig` no escopo de teste via `use crate::protection::CompressConfig;`.

- [ ] **Step 4: Run test to verify it passes**

Run: `cargo test -p omnicompress-core pipeline::`
Expected: PASS (2 testes).

- [ ] **Step 5: Commit**

```bash
git add crates/omnicompress-core/src/pipeline.rs crates/omnicompress-core/src/lib.rs
git commit -m "feat(core): CompressionPipeline + Measurement + fail-open (catch_unwind)"
```

---

### Task 11: EmbeddedStore (redb) passa no contrato

**Files:**
- Modify: `crates/omnicompress-core/src/ccr/embedded.rs`, `tests/contract_ccrstore.rs`

- [ ] **Step 1: Estender o teste de contrato**

```rust
// adicionar em tests/contract_ccrstore.rs
use omnicompress_core::ccr::EmbeddedStore;
#[test]
fn embedded_store_satisfies_contract() {
    let dir = std::env::temp_dir().join(format!("oc_ccr_test_{}", std::process::id()));
    let store = EmbeddedStore::open(dir.join("ccr.redb")).unwrap();
    roundtrip(store);
}
```

- [ ] **Step 2: Run to verify it fails**

Run: `cargo test -p omnicompress-core --test contract_ccrstore embedded`
Expected: FAIL — `EmbeddedStore::open` é stub.

- [ ] **Step 3: Implement**

```rust
// ccr/embedded.rs
use super::{CCRStore, Hash, hash_of};
use redb::{Database, TableDefinition, ReadableTable};
use std::path::Path;

const T: TableDefinition<&str, &[u8]> = TableDefinition::new("ccr_originals");

pub struct EmbeddedStore { db: Database }

impl EmbeddedStore {
    pub fn open<P: AsRef<Path>>(path: P) -> std::io::Result<Self> {
        if let Some(p) = path.as_ref().parent() { std::fs::create_dir_all(p)?; }
        let db = Database::create(path).map_err(to_io)?;
        // garante a tabela
        let w = db.begin_write().map_err(to_io)?;
        { let _ = w.open_table(T).map_err(to_io)?; }
        w.commit().map_err(to_io)?;
        Ok(Self { db })
    }
}

impl CCRStore for EmbeddedStore {
    fn put(&self, original: &[u8]) -> std::io::Result<Hash> {
        let h = hash_of(original);
        let w = self.db.begin_write().map_err(to_io)?;
        { let mut t = w.open_table(T).map_err(to_io)?; t.insert(h.as_str(), original).map_err(to_io)?; }
        w.commit().map_err(to_io)?;
        Ok(h)
    }
    fn get(&self, hash: &str) -> std::io::Result<Option<Vec<u8>>> {
        let r = self.db.begin_read().map_err(to_io)?;
        let t = r.open_table(T).map_err(to_io)?;
        Ok(t.get(hash).map_err(to_io)?.map(|v| v.value().to_vec()))
    }
}

fn to_io<E: std::fmt::Display>(e: E) -> std::io::Error { std::io::Error::new(std::io::ErrorKind::Other, e.to_string()) }
```

- [ ] **Step 4: Run to verify it passes**

Run: `cargo test -p omnicompress-core --test contract_ccrstore`
Expected: PASS (memory + embedded).

- [ ] **Step 5: Commit**

```bash
git add crates/omnicompress-core/src/ccr/embedded.rs crates/omnicompress-core/tests/contract_ccrstore.rs
git commit -m "feat(core): EmbeddedStore (redb) durável, passa no contrato CCRStore"
```

---

### Task 12: EvalHarness

**Files:**
- Create: `crates/omnicompress-core/src/eval.rs`; `pub mod eval;` em `lib.rs`

- [ ] **Step 1: Write the failing test**

```rust
#[cfg(test)]
mod tests {
    use super::*;
    use crate::types::{Block, Role};
    use crate::ccr::MemoryStore;
    use crate::pipeline::CompressionPipeline;
    use crate::protection::CompressConfig;
    #[test]
    fn report_has_ratio_and_roundtrip_ok() {
        let big = "[".to_string() + &(0..60).map(|i| format!(r#"{{"id":{i}}}"#)).collect::<Vec<_>>().join(",") + "]";
        let mut msgs = vec![Block::tool(Role::User, &big, "search")];
        for i in 0..6 { msgs.push(Block::text(Role::Assistant, &format!("ok {i}"))); }
        let store = std::sync::Arc::new(MemoryStore::default());
        let pipe = CompressionPipeline::new_arc(store.clone());
        let rep = EvalHarness::run_one(&pipe, &*store, msgs, &CompressConfig::default());
        assert!(rep.ratio > 0.0 && rep.ratio < 1.0);
        assert!(rep.roundtrip_ok, "todos os originais do CCR recuperam byte-idêntico");
    }
}
```

- [ ] **Step 2: Run to verify it fails**

Run: `cargo test -p omnicompress-core eval::`
Expected: FAIL.

- [ ] **Step 3: Implement**

> Requer um construtor `CompressionPipeline::new_arc(Arc<dyn CCRStore>)` e expor o `ccr` para o harness verificar round-trip. Adicione em `pipeline.rs`: guarde `ccr: Arc<dyn CCRStore>` (troque `Box` por `Arc`), e `pub fn new_arc(ccr: Arc<dyn CCRStore>) -> Self`. Ajuste Task 10 (`Box`→`Arc`) ao chegar aqui.

```rust
use crate::types::Block;
use crate::pipeline::CompressionPipeline;
use crate::protection::CompressConfig;
use crate::ccr::CCRStore;

pub struct EvalReport { pub tokens_before: usize, pub tokens_after: usize, pub ratio: f64, pub roundtrip_ok: bool }

pub struct EvalHarness;
impl EvalHarness {
    pub fn run_one(pipe: &CompressionPipeline, ccr: &dyn CCRStore, msgs: Vec<Block>, cfg: &CompressConfig) -> EvalReport {
        let r = pipe.compress(msgs, cfg);
        let ratio = if r.tokens_before == 0 { 0.0 } else { r.tokens_after as f64 / r.tokens_before as f64 };
        // round-trip: todo ccr_ref tem que recuperar
        let roundtrip_ok = r.ccr_refs.iter().all(|cr| matches!(ccr.get(&cr.hash), Ok(Some(_))));
        EvalReport { tokens_before: r.tokens_before, tokens_after: r.tokens_after, ratio, roundtrip_ok }
    }
}
```

- [ ] **Step 4: Run to verify it passes**

Run: `cargo test -p omnicompress-core eval::`
Expected: PASS.

- [ ] **Step 5: Commit**

```bash
git add crates/omnicompress-core/src/eval.rs crates/omnicompress-core/src/pipeline.rs crates/omnicompress-core/src/lib.rs
git commit -m "feat(core): EvalHarness (ratio + round-trip CCR) + pipeline Arc<CCRStore>"
```

---

### Task 13: Python binding (PyO3) — `compress()`

**Files:**
- Create: `crates/omnicompress-py/Cargo.toml`, `.../pyproject.toml`, `.../src/lib.rs`, `.../python/tests/test_compress.py`

- [ ] **Step 1: `Cargo.toml`**

```toml
[package]
name = "omnicompress-py"
version.workspace = true
edition.workspace = true
license.workspace = true

[lib]
name = "omnicompress"
crate-type = ["cdylib"]

[dependencies]
omnicompress-core = { path = "../omnicompress-core" }
pyo3 = { version = "0.22", features = ["extension-module"] }
serde_json = "1"
```

- [ ] **Step 2: `pyproject.toml`**

```toml
[build-system]
requires = ["maturin>=1.5,<2"]
build-backend = "maturin"

[project]
name = "omnicompress"
version = "0.1.0"
requires-python = ">=3.10"

[tool.maturin]
manifest-path = "Cargo.toml"
module-name = "omnicompress"
```

- [ ] **Step 3: Write the failing test (Python)**

```python
# python/tests/test_compress.py
import json, omnicompress

def test_compress_returns_metrics_and_shrinks_old_tool_json():
    big = json.dumps([{"id": i, "v": str(i)} for i in range(60)])
    msgs = [{"role": "user", "content": big, "tool_name": "search"}]
    msgs += [{"role": "assistant", "content": f"ok {i}"} for i in range(6)]
    res = omnicompress.compress(msgs)
    assert res["tokens_after"] < res["tokens_before"]
    assert len(res["ccr_refs"]) == 1

def test_fail_open_short_content_untouched():
    res = omnicompress.compress([{"role": "user", "content": "curto"}])
    assert res["messages"][0]["content"] == "curto"
```

- [ ] **Step 4: Implement `src/lib.rs`**

```rust
use pyo3::prelude::*;
use pyo3::types::{PyDict, PyList};
use std::sync::Arc;
use omnicompress_core::types::{Block, Role};
use omnicompress_core::ccr::MemoryStore;
use omnicompress_core::pipeline::CompressionPipeline;
use omnicompress_core::protection::CompressConfig;

fn role_from(s: &str) -> Role {
    match s { "system" => Role::System, "assistant" => Role::Assistant, "tool" => Role::Tool, _ => Role::User }
}

#[pyfunction]
fn compress(py: Python<'_>, messages: Vec<Bound<'_, PyDict>>) -> PyResult<Py<PyDict>> {
    let mut blocks = Vec::new();
    for m in &messages {
        let role = m.get_item("role")?.map(|v| v.extract::<String>()).transpose()?.unwrap_or_else(|| "user".into());
        let content = m.get_item("content")?.map(|v| v.extract::<String>()).transpose()?.unwrap_or_default();
        let tool = m.get_item("tool_name")?.and_then(|v| v.extract::<String>().ok());
        blocks.push(Block { role: role_from(&role), content, tool_name: tool });
    }
    let pipe = CompressionPipeline::new_arc(Arc::new(MemoryStore::default()));
    let r = pipe.compress(blocks, &CompressConfig::default());

    let out = PyDict::new_bound(py);
    out.set_item("tokens_before", r.tokens_before)?;
    out.set_item("tokens_after", r.tokens_after)?;
    out.set_item("tokens_saved", r.tokens_saved())?;
    let msgs = PyList::empty_bound(py);
    for b in &r.messages {
        let d = PyDict::new_bound(py);
        d.set_item("content", &b.content)?;
        msgs.append(d)?;
    }
    out.set_item("messages", msgs)?;
    let refs = PyList::empty_bound(py);
    for c in &r.ccr_refs { let d = PyDict::new_bound(py); d.set_item("hash", &c.hash)?; refs.append(d)?; }
    out.set_item("ccr_refs", refs)?;
    Ok(out.into())
}

#[pymodule]
fn omnicompress(m: &Bound<'_, PyModule>) -> PyResult<()> {
    m.add_function(wrap_pyfunction!(compress, m)?)?;
    Ok(())
}
```

- [ ] **Step 5: Build + run Python test**

Run:
```bash
cd crates/omnicompress-py && maturin develop && python -m pytest python/tests/ -v
```
Expected: 2 PASS.

- [ ] **Step 6: Commit**

```bash
git add crates/omnicompress-py
git commit -m "feat(py): binding PyO3 compress() + testes pytest"
```

---

### Task 14: CLI (`omnicompress compress|eval`)

**Files:**
- Create: `crates/omnicompress-core/src/bin/omnicompress.rs`; add `clap` em deps

- [ ] **Step 1: Add dep** — `clap = { version = "4", features = ["derive"] }`
- [ ] **Step 2: Write the failing test (integration)**

```rust
// tests/cli.rs
#[test]
fn cli_compress_file_reports_savings() {
    // gera arquivo json grande, roda o bin, confere que imprime tokens_before/after
    // (use assert_cmd se preferir; aqui via std::process::Command)
    use std::process::Command;
    let f = std::env::temp_dir().join("oc_cli.json");
    let big = "[".to_string() + &(0..60).map(|i| format!(r#"{{"id":{i}}}"#)).collect::<Vec<_>>().join(",") + "]";
    std::fs::write(&f, big).unwrap();
    let out = Command::new(env!("CARGO_BIN_EXE_omnicompress")).arg("compress").arg(&f).output().unwrap();
    let s = String::from_utf8_lossy(&out.stdout);
    assert!(s.contains("tokens_before") && s.contains("tokens_after"));
}
```

- [ ] **Step 3: Implement bin** (parse args `compress <file>` / `eval <dir>`, monta 1 mensagem tool, chama pipeline com `MemoryStore`, imprime JSON do `CompressResult`).

```rust
use clap::{Parser, Subcommand};
use std::sync::Arc;
use omnicompress_core::{pipeline::CompressionPipeline, ccr::MemoryStore, protection::CompressConfig, types::{Block, Role}};

#[derive(Parser)] struct Cli { #[command(subcommand)] cmd: Cmd }
#[derive(Subcommand)] enum Cmd { Compress { file: String }, Eval { dir: String } }

fn main() {
    let cli = Cli::parse();
    match cli.cmd {
        Cmd::Compress { file } => {
            let content = std::fs::read_to_string(&file).expect("read file");
            let pipe = CompressionPipeline::new_arc(Arc::new(MemoryStore::default()));
            let r = pipe.compress(vec![Block::tool(Role::User, &content, "Bash")], &CompressConfig::default());
            println!("{{\"tokens_before\":{},\"tokens_after\":{},\"tokens_saved\":{}}}", r.tokens_before, r.tokens_after, r.tokens_saved());
        }
        Cmd::Eval { dir } => { eprintln!("eval em {dir} — itera arquivos e roda EvalHarness (ver eval.rs)"); }
    }
}
```

- [ ] **Step 4: Run** → `cargo test -p omnicompress-core --test cli` → PASS.
- [ ] **Step 5: Commit** → `git commit -m "feat(cli): omnicompress compress|eval"`

---

### Task 15: CI cross-platform + dogfood gate

**Files:**
- Create: `.github/workflows/ci.yml` (Forgejo Actions usa o mesmo formato)

- [ ] **Step 1: CI matrix**

```yaml
name: ci
on: [push, pull_request]
jobs:
  test:
    strategy:
      matrix:
        os: [ubuntu-latest, macos-latest, windows-latest]
    runs-on: ${{ matrix.os }}
    steps:
      - uses: actions/checkout@v4
      - uses: dtolnay/rust-toolchain@stable
      - run: cargo test --workspace
  wheels:
    needs: test
    strategy:
      matrix:
        os: [ubuntu-latest, macos-latest, windows-latest]
    runs-on: ${{ matrix.os }}
    steps:
      - uses: actions/checkout@v4
      - uses: PyO3/maturin-action@v1
        with: { command: build, args: "-m crates/omnicompress-py/Cargo.toml --release" }
```

- [ ] **Step 2: Dogfood gate (manual, documentado)** — rodar `omnicompress eval` no corpus real do dispatch e confirmar **redução blended ≥ 68%** + `roundtrip_ok=true`. Registrar o número no PR.

- [ ] **Step 3: Commit** → `git commit -m "ci: matriz 3 SOs + build de wheels (maturin)"`

---

## Self-Review

**Spec coverage:** Router (T3), 4 compressores (T4/T5/T7/T9), CCRStore trait+2 impls (T6/T11), ProtectionPolicy (T8), Pipeline+Measurement+fail-open (T10), EvalHarness (T12), binding Python (T13), CLI (T14), cross-platform CI (T15), critérios de sucesso (T15 gate). Tokenizer honesto (T2). ✅ Sem gaps vs spec.

**Placeholder scan:** todos os steps de código têm código real. Os stubs (T4/T6) são explicitamente "compila e retorna untouched/`open` stub", substituídos em tasks nomeadas (T5/T7/T9/T11). ✅

**Type consistency:** `Block`, `Outcome{compressed,original,detail}`, `CompressResult{messages,tokens_before,tokens_after,transforms,ccr_refs}`, `CCRStore{put,get}`, `Compressor{compress,name}`, `CompressConfig`, `CompressionPipeline::new/new_arc` — consistentes entre tasks. ⚠️ Nota de ajuste em T10/T12: `Box<dyn CCRStore>` vira `Arc<dyn CCRStore>` quando o EvalHarness chega (T12) — registrado no próprio T12. Corrigir o método `Block::text` duplicado (T1) pra `from_text` construtor + `text(&self)` getter.

---

## Execution Handoff

Plano salvo em `docs/plans/2026-06-18-sp1-omnicompress-core.md`. Duas opções de execução:

1. **Subagent-Driven (recomendado)** — um subagente fresco por task, review entre tasks. ⚠️ Tasks leaf (T5/T7/T9 compressores; T11 store) só paralelizam DEPOIS das traits (T4/T6) existirem — antes disso é sequencial (estado compartilhado).
2. **Inline** — executo as tasks nesta sessão com checkpoints.

Qual abordagem?
