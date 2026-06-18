use pyo3::prelude::*;
use pyo3::types::{PyDict, PyList};
use std::sync::Arc;

use omnicompress_core::ccr::MemoryStore;
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
// The `useless_conversion` lint fires on `#[pyfunction]` macro-generated code, not ours.
#[allow(clippy::useless_conversion)]
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

/// omnicompress Python extension module.
#[pymodule]
fn omnicompress(m: &Bound<'_, PyModule>) -> PyResult<()> {
    m.add_function(wrap_pyfunction!(compress, m)?)?;
    Ok(())
}
