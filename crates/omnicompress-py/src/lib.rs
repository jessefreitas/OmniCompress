// The `useless_conversion` lint fires inside `#[pyfunction]` macro-generated code, not ours.
#![allow(clippy::useless_conversion)]

use pyo3::prelude::*;
use pyo3::exceptions::PyIOError;
use pyo3::types::{PyDict, PyList};
use std::sync::Arc;

use omnicompress_core::ccr::{CCRStore, EmbeddedStore, MemoryStore};
use omnicompress_core::pipeline::CompressionPipeline;
use omnicompress_core::protection::CompressConfig;
use omnicompress_core::types::{Block, Role};

fn role_from(s: &str) -> Role {
    match s {
        "system" => Role::System,
        "assistant" => Role::Assistant,
        "tool" => Role::Tool,
        _ => Role::User,
    }
}

/// Build a [`CompressConfig`] from optional Python overrides, falling back to the
/// library defaults (lossless=true, cache_stable=false) for anything not given.
fn build_config(
    lossless: bool,
    cache_stable: bool,
    protect_recent: Option<usize>,
    min_chars_to_compress: Option<usize>,
) -> CompressConfig {
    let d = CompressConfig::default();
    CompressConfig {
        protect_recent: protect_recent.unwrap_or(d.protect_recent),
        min_chars_to_compress: min_chars_to_compress.unwrap_or(d.min_chars_to_compress),
        excluded_tools: d.excluded_tools,
        cache_stable,
        lossless,
    }
}

/// Convert a list of `{role, content, tool_name?}` dicts into core `Block`s.
fn messages_to_blocks(messages: &[Bound<'_, PyDict>]) -> PyResult<Vec<Block>> {
    let mut blocks: Vec<Block> = Vec::with_capacity(messages.len());
    for m in messages {
        let role: String = m
            .get_item("role")?
            .map(|v| v.extract::<String>())
            .transpose()?
            .unwrap_or_else(|| "user".to_string());

        let content: String = m
            .get_item("content")?
            .map(|v| v.extract::<String>())
            .transpose()?
            .unwrap_or_default();

        let tool_name: Option<String> = m.get_item("tool_name")?.and_then(|v| {
            // treat Python None as no tool_name
            if v.is_none() {
                None
            } else {
                v.extract::<String>().ok()
            }
        });

        blocks.push(Block {
            role: role_from(&role),
            content,
            tool_name,
        });
    }
    Ok(blocks)
}

/// Serialise a core `CompressResult` into the public Python dict shape.
fn result_to_pydict(
    py: Python<'_>,
    r: &omnicompress_core::types::CompressResult,
) -> PyResult<Py<PyDict>> {
    let out = PyDict::new_bound(py);
    out.set_item("tokens_before", r.tokens_before)?;
    out.set_item("tokens_after", r.tokens_after)?;
    out.set_item("tokens_saved", r.tokens_saved())?;

    let msgs_list = PyList::empty_bound(py);
    for b in &r.messages {
        let d = PyDict::new_bound(py);
        d.set_item("content", &b.content)?;
        msgs_list.append(d)?;
    }
    out.set_item("messages", msgs_list)?;

    let refs_list = PyList::empty_bound(py);
    for c in &r.ccr_refs {
        let d = PyDict::new_bound(py);
        d.set_item("hash", &c.hash)?;
        refs_list.append(d)?;
    }
    out.set_item("ccr_refs", refs_list)?;

    Ok(out.into())
}

/// Compress a list of message dicts.
///
/// Each dict must have:
///   - "role": str ("user" | "assistant" | "system" | "tool")
///   - "content": str
///   - "tool_name": str | None  (optional)
///
/// Compression behaviour (all optional — defaults match the Rust library):
///   - `lossless` (default **True**): reversible transforms only (e.g. a JSON
///     array becomes a compact columnar table keeping every row). Nothing is sent
///     to the CCR, so the model can answer from the compressed view alone — no
///     retrieve loop needed. Set **False** for aggressive lossy compression
///     (sampling/elision) with the original stored in the CCR for retrieval.
///   - `cache_stable` (default **False**): when True, a block's compressed form
///     depends only on its content, never on its position, so the prompt prefix
///     stays byte-stable across turns and the provider's cache keeps hitting.
///     The library default is **False** (the recent-N window is protected); the
///     bundled proxy opts into True. Enable it when you front a prompt cache.
///   - `protect_recent`: trailing messages to always keep verbatim (default 4).
///   - `min_chars_to_compress`: minimum size before compression is attempted
///     (default 600).
///
/// Returns a dict with keys:
///   - "tokens_before": int
///   - "tokens_after":  int
///   - "tokens_saved":  int
///   - "messages": list[dict{"content": str}]
///   - "ccr_refs": list[dict{"hash": str}]
#[pyfunction]
#[pyo3(signature = (messages, *, lossless=true, cache_stable=false, protect_recent=None, min_chars_to_compress=None))]
fn compress(
    py: Python<'_>,
    messages: Vec<Bound<'_, PyDict>>,
    lossless: bool,
    cache_stable: bool,
    protect_recent: Option<usize>,
    min_chars_to_compress: Option<usize>,
) -> PyResult<Py<PyDict>> {
    let blocks = messages_to_blocks(&messages)?;
    let cfg = build_config(lossless, cache_stable, protect_recent, min_chars_to_compress);

    let store = Arc::new(MemoryStore::default());
    let pipe = CompressionPipeline::new_arc(store);
    let r = pipe.compress(blocks, &cfg);

    result_to_pydict(py, &r)
}

/// A stateful omnicompress session that keeps a persistent CCR store.
///
/// The same store is shared between `compress` (which stores references) and
/// `retrieve` (which fetches them back).
#[pyclass(name = "OmniCompressSession")]
struct PySession {
    ccr: Arc<dyn CCRStore>,
    pipeline: CompressionPipeline,
}

#[pymethods]
impl PySession {
    /// Create a new session.
    ///
    /// If `ccr_path` is provided, an [`EmbeddedStore`] is opened from that path.
    /// Otherwise an in-memory [`MemoryStore`] is used.
    #[new]
    #[pyo3(signature = (ccr_path=None))]
    fn new(ccr_path: Option<String>) -> PyResult<Self> {
        let ccr: Arc<dyn CCRStore> = if let Some(path) = ccr_path {
            let store = EmbeddedStore::open(path)
                .map_err(|e| PyIOError::new_err(format!("failed to open embedded store: {e}")))?;
            Arc::new(store)
        } else {
            Arc::new(MemoryStore::default())
        };
        let pipeline = CompressionPipeline::new_arc(ccr.clone());
        Ok(Self { ccr, pipeline })
    }

    /// Compress a list of message dicts using this session's persistent pipeline.
    ///
    /// Accepts the same optional config as the free `compress` function
    /// (`lossless`, `cache_stable`, `protect_recent`, `min_chars_to_compress`).
    /// In aggressive mode (`lossless=False`) originals are stored in THIS
    /// session's CCR and can be recovered with `retrieve`.
    #[pyo3(signature = (messages, *, lossless=true, cache_stable=false, protect_recent=None, min_chars_to_compress=None))]
    fn compress(
        &self,
        py: Python<'_>,
        messages: Vec<Bound<'_, PyDict>>,
        lossless: bool,
        cache_stable: bool,
        protect_recent: Option<usize>,
        min_chars_to_compress: Option<usize>,
    ) -> PyResult<Py<PyDict>> {
        let blocks = messages_to_blocks(&messages)?;
        let cfg = build_config(lossless, cache_stable, protect_recent, min_chars_to_compress);
        let r = self.pipeline.compress(blocks, &cfg);
        result_to_pydict(py, &r)
    }

    /// Retrieve the original content stored under a given CCR hash.
    ///
    /// Returns `None` if the hash is not found or if retrieval fails (fail-open).
    fn retrieve(&self, hash: &str) -> PyResult<Option<String>> {
        match self.ccr.get(hash) {
            Ok(Some(bytes)) => Ok(Some(String::from_utf8_lossy(&bytes).into_owned())),
            Ok(None) => Ok(None),
            Err(_) => Ok(None),
        }
    }
}

/// omnicompress Python extension module.
#[pymodule]
fn omnicompress(m: &Bound<'_, PyModule>) -> PyResult<()> {
    m.add_function(wrap_pyfunction!(compress, m)?)?;
    m.add_class::<PySession>()?;
    Ok(())
}
