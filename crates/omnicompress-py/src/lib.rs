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

/// Compress a list of message dicts.
///
/// Each dict must have:
///   - "role": str ("user" | "assistant" | "system" | "tool")
///   - "content": str
///   - "tool_name": str | None  (optional)
///
/// Returns a dict with keys:
///   - "tokens_before": int
///   - "tokens_after":  int
///   - "tokens_saved":  int
///   - "messages": list[dict{"content": str}]
///   - "ccr_refs": list[dict{"hash": str}]
#[pyfunction]
fn compress(py: Python<'_>, messages: Vec<Bound<'_, PyDict>>) -> PyResult<Py<PyDict>> {
    let mut blocks: Vec<Block> = Vec::with_capacity(messages.len());

    for m in &messages {
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

        let tool_name: Option<String> = m
            .get_item("tool_name")?
            .and_then(|v| {
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

    let store = Arc::new(MemoryStore::default());
    let pipe = CompressionPipeline::new_arc(store);
    let r = pipe.compress(blocks, &CompressConfig::default());

    // Build the output dict
    let out = PyDict::new_bound(py);
    out.set_item("tokens_before", r.tokens_before)?;
    out.set_item("tokens_after", r.tokens_after)?;
    out.set_item("tokens_saved", r.tokens_saved())?;

    // messages list
    let msgs_list = PyList::empty_bound(py);
    for b in &r.messages {
        let d = PyDict::new_bound(py);
        d.set_item("content", &b.content)?;
        msgs_list.append(d)?;
    }
    out.set_item("messages", msgs_list)?;

    // ccr_refs list
    let refs_list = PyList::empty_bound(py);
    for c in &r.ccr_refs {
        let d = PyDict::new_bound(py);
        d.set_item("hash", &c.hash)?;
        refs_list.append(d)?;
    }
    out.set_item("ccr_refs", refs_list)?;

    Ok(out.into())
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
    /// See the free `compress` function for the message/return schema.
    fn compress(
        &self,
        py: Python<'_>,
        messages: Vec<Bound<'_, PyDict>>,
    ) -> PyResult<Py<PyDict>> {
        let mut blocks: Vec<Block> = Vec::with_capacity(messages.len());

        for m in &messages {
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

            let tool_name: Option<String> = m
                .get_item("tool_name")?
                .and_then(|v| {
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

        let r = self.pipeline.compress(blocks, &CompressConfig::default());

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
