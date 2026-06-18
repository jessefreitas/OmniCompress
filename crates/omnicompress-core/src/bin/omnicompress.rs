use clap::{Parser, Subcommand};
use std::sync::Arc;

use omnicompress_core::ccr::MemoryStore;
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
    /// Evaluate compression quality over a directory of samples (stub).
    Eval {
        /// Directory containing sample files.
        dir: String,
    },
}

fn main() {
    let cli = Cli::parse();
    match cli.cmd {
        Cmd::Compress { file } => {
            let content = std::fs::read_to_string(&file)
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
        Cmd::Eval { dir } => {
            eprintln!(
                "eval over {dir} — iterates sample files and runs EvalHarness (see eval.rs, stub for now)"
            );
        }
    }
}
