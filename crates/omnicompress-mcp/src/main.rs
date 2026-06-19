//! `omnicompress-mcp` binary — MCP server speaking JSON-RPC 2.0 over stdio.
//!
//! Reads one JSON-RPC request per line on stdin and writes one response per
//! line on stdout. State (pipeline + CCR store) persists for the session, so
//! `omnicompress_retrieve` can return originals stored by `omnicompress_compress`.

use omnicompress_mcp::{handle_jsonrpc, McpState};
use std::io::{self, BufRead, Write};

fn main() {
    let state = McpState::new();
    let stdin = io::stdin();
    let mut stdout = io::stdout();
    for line in stdin.lock().lines() {
        let Ok(line) = line else { break };
        if line.trim().is_empty() {
            continue;
        }
        let response = handle_jsonrpc(&line, &state);
        let _ = writeln!(stdout, "{response}");
        let _ = stdout.flush();
    }
}
