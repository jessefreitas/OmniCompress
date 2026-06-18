use clap::{Parser, Subcommand};
use std::sync::Arc;

use omnicompress_core::ccr::MemoryStore;
use omnicompress_core::eval::EvalHarness;
use omnicompress_core::pipeline::CompressionPipeline;
use omnicompress_core::protection::CompressConfig;
use omnicompress_core::types::{Block, Role};

#[derive(Parser)]
#[command(name = "omnicompress", about = "Context compression engine")]
struct Cli {
    #[command(subcommand)]
    cmd: Cmd,
}

#[derive(Subcommand)]
enum Cmd {
    /// Compress a file containing raw content (reads as a single tool block).
    Compress {
        /// Path to the file to compress.
        file: String,
    },
    /// Evaluate compression quality over a directory of JSON sample files.
    ///
    /// Each `*.json` file in `<dir>` must contain a JSON array of Block objects
    /// (`[{"role": ..., "content": ..., "tool_name": ...}, ...]`).
    /// Prints a per-file + aggregate summary in JSON.
    Eval {
        /// Directory containing sample `*.json` files (each a `Vec<Block>`).
        dir: String,
    },
}

fn main() {
    let cli = Cli::parse();
    match cli.cmd {
        Cmd::Compress { file } => run_compress(&file),
        Cmd::Eval { dir } => run_eval(&dir),
    }
}

fn run_compress(file: &str) {
    let content = std::fs::read_to_string(file)
        .unwrap_or_else(|e| { eprintln!("error reading {file}: {e}"); std::process::exit(1); });

    let store = Arc::new(MemoryStore::default());
    let pipe = CompressionPipeline::new_arc(store);
    // Wrap the file content as a single Bash tool block at position 0
    // with several trailing assistant blocks so it exits the recent window.
    let mut msgs = vec![Block::tool(Role::User, &content, "Bash")];
    for i in 0..6 {
        msgs.push(Block::from_text(Role::Assistant, &format!("ok {i}")));
    }
    let r = pipe.compress(msgs, &CompressConfig::default());

    println!(
        "{{\"tokens_before\":{},\"tokens_after\":{},\"tokens_saved\":{}}}",
        r.tokens_before,
        r.tokens_after,
        r.tokens_saved()
    );
}

fn run_eval(dir: &str) {
    let read_dir = std::fs::read_dir(dir).unwrap_or_else(|e| {
        eprintln!("error reading directory {dir}: {e}");
        std::process::exit(1);
    });

    let mut files: Vec<std::path::PathBuf> = read_dir
        .filter_map(|e| e.ok().map(|e| e.path()))
        .filter(|p| p.extension().and_then(|s| s.to_str()) == Some("json"))
        .collect();
    files.sort();

    if files.is_empty() {
        println!("{{\"files\":[],\"aggregate\":{{\"tokens_before\":0,\"tokens_after\":0,\"ratio\":null,\"roundtrip_ok\":true}}}}");
        return;
    }

    let cfg = CompressConfig::default();
    let mut file_reports: Vec<serde_json::Value> = Vec::new();
    let mut total_before: usize = 0;
    let mut total_after: usize = 0;
    let mut all_roundtrip_ok = true;

    for path in &files {
        let raw = match std::fs::read_to_string(path) {
            Ok(s) => s,
            Err(e) => {
                eprintln!("warning: skipping {}: {e}", path.display());
                continue;
            }
        };

        let msgs: Vec<Block> = match serde_json::from_str(&raw) {
            Ok(v) => v,
            Err(e) => {
                eprintln!("warning: skipping {} (parse error): {e}", path.display());
                continue;
            }
        };

        let store = Arc::new(MemoryStore::default());
        let pipe = CompressionPipeline::new_arc(store.clone());
        let report = EvalHarness::run_one(&pipe, &*store, msgs, &cfg);

        total_before += report.tokens_before;
        total_after += report.tokens_after;
        if !report.roundtrip_ok {
            all_roundtrip_ok = false;
        }

        file_reports.push(serde_json::json!({
            "file": path.file_name().and_then(|n| n.to_str()).unwrap_or("?"),
            "tokens_before": report.tokens_before,
            "tokens_after": report.tokens_after,
            "ratio": report.ratio,
            "roundtrip_ok": report.roundtrip_ok,
        }));
    }

    let agg_ratio: serde_json::Value = if total_before == 0 {
        serde_json::Value::Null
    } else {
        serde_json::json!(total_after as f64 / total_before as f64)
    };

    let summary = serde_json::json!({
        "files": file_reports,
        "aggregate": {
            "tokens_before": total_before,
            "tokens_after": total_after,
            "ratio": agg_ratio,
            "roundtrip_ok": all_roundtrip_ok,
        }
    });

    println!("{}", serde_json::to_string_pretty(&summary).unwrap());
}
