//! `omnicompress-proxy` binary — a drop-in compressing proxy.
//!
//! Env:
//!   OMNICOMPRESS_UPSTREAM  upstream base URL (default https://api.openai.com)
//!   OMNICOMPRESS_BIND      bind address (default 127.0.0.1:8787)

use std::net::SocketAddr;

#[tokio::main]
async fn main() {
    let upstream = std::env::var("OMNICOMPRESS_UPSTREAM")
        .unwrap_or_else(|_| "https://api.openai.com".to_string());
    let addr: SocketAddr = std::env::var("OMNICOMPRESS_BIND")
        .ok()
        .and_then(|s| s.parse().ok())
        .unwrap_or_else(|| "127.0.0.1:8787".parse().expect("valid default addr"));

    eprintln!("omnicompress-proxy → {upstream} listening on http://{addr}");
    omnicompress_proxy::server::serve(upstream, addr).await;
}
