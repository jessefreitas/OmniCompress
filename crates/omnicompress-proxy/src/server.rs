//! HTTP proxy server (Axum) wrapping the wire-compression functions.
//!
//! Drop-in proxy: forwards `POST /v1/chat/completions` (OpenAI wire) and
//! `POST /v1/messages` (Anthropic wire) to a configurable upstream, compressing
//! the request body first. Fail-open: compression panics fall back to the
//! original body; upstream/network failures return HTTP 502 (never 500).

use axum::{
    body::{Body, Bytes},
    extract::State,
    http::{header::CONTENT_TYPE, HeaderMap, HeaderValue, StatusCode},
    response::Response,
    routing::post,
    Router,
};
use std::net::SocketAddr;
use std::panic::AssertUnwindSafe;

/// Build the proxy router with the given upstream base URL as state.
pub fn app(upstream: String) -> Router {
    Router::new()
        .route("/v1/chat/completions", post(openai_handler))
        .route("/v1/messages", post(anthropic_handler))
        .with_state(upstream)
}

/// Core proxy logic shared by both handlers.
///
/// - Compresses the incoming body using `compress` (fail-open via `catch_unwind`).
/// - Forwards the (possibly compressed) body to `{upstream}{path}`.
/// - On any upstream/network failure, returns HTTP 502 with an empty body.
async fn proxy(
    upstream: String,
    path: &str,
    mut headers: HeaderMap,
    body: Bytes,
    compress: fn(&[u8]) -> Vec<u8>,
    shape: fn(&[u8]) -> Vec<u8>,
) -> Response {
    let compressed: Vec<u8> = {
        let body_ref: &[u8] = body.as_ref();
        match std::panic::catch_unwind(AssertUnwindSafe(|| compress(body_ref))) {
            Ok(c) => c,
            Err(_) => body_ref.to_vec(),
        }
    };

    // Output-shaping (opt-in via OMNICOMPRESS_OUTPUT_SHAPER): steer the model
    // toward terser output to cut output tokens. Deterministic + stable across
    // turns, so it doesn't bust the provider's prefix cache.
    let outgoing: Vec<u8> = if crate::shaper::enabled() {
        std::panic::catch_unwind(AssertUnwindSafe(|| shape(&compressed)))
            .unwrap_or(compressed)
    } else {
        compressed
    };

    // Forward the client's headers (Authorization, x-api-key, anthropic-version, …)
    // to the upstream — a proxy that dropped auth would always get 401. Strip the
    // headers reqwest manages itself (host, content-length).
    headers.remove("host");
    headers.remove("content-length");
    if !headers.contains_key(CONTENT_TYPE) {
        headers.insert(CONTENT_TYPE, HeaderValue::from_static("application/json"));
    }

    let client = reqwest::Client::new();
    let send_result = client
        .post(format!("{upstream}{path}"))
        .headers(headers)
        .body(outgoing)
        .send()
        .await;

    let upstream_response = match send_result {
        Ok(r) => r,
        Err(_) => {
            return Response::builder()
                .status(StatusCode::BAD_GATEWAY)
                .body(Body::empty())
                .expect("static response is valid");
        }
    };

    let status_code =
        StatusCode::from_u16(upstream_response.status().as_u16()).unwrap_or(StatusCode::BAD_GATEWAY);
    let bytes = upstream_response.bytes().await.unwrap_or_default();

    Response::builder()
        .status(status_code)
        .body(Body::from(bytes))
        .unwrap_or_else(|_| {
            Response::builder()
                .status(StatusCode::BAD_GATEWAY)
                .body(Body::empty())
                .expect("static fallback response is valid")
        })
}

/// Handler for `POST /v1/chat/completions` (OpenAI wire format).
async fn openai_handler(
    State(upstream): State<String>,
    headers: HeaderMap,
    body: Bytes,
) -> Response {
    proxy(
        upstream,
        "/v1/chat/completions",
        headers,
        body,
        crate::wire::compress_openai_request,
        crate::shaper::shape_openai,
    )
    .await
}

/// Handler for `POST /v1/messages` (Anthropic wire format).
async fn anthropic_handler(
    State(upstream): State<String>,
    headers: HeaderMap,
    body: Bytes,
) -> Response {
    proxy(
        upstream,
        "/v1/messages",
        headers,
        body,
        crate::wire::compress_anthropic_request,
        crate::shaper::shape_anthropic,
    )
    .await
}

/// Bind a TCP listener on `addr` and serve the proxy application.
pub async fn serve(upstream: String, addr: SocketAddr) {
    let listener = tokio::net::TcpListener::bind(addr)
        .await
        .expect("failed to bind listener");
    axum::serve(listener, app(upstream))
        .await
        .expect("server failed");
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::{json, Value};
    use tower::ServiceExt;
    use wiremock::{matchers::method, Mock, MockServer, ResponseTemplate};

    /// A request body whose FIRST message carries a large, compressible
    /// JSON-array string (>= 20 records) — so the wire compressor actually fires —
    /// followed by short recent messages. `extra` fields are merged at the top level.
    fn compressible_body(extra: Value) -> Vec<u8> {
        let arr: Vec<Value> = (0..60).map(|i| json!({"id": i, "v": i.to_string()})).collect();
        let big = serde_json::to_string(&arr).unwrap();
        let mut messages = vec![json!({"role": "user", "content": big})];
        for i in 0..6 {
            messages.push(json!({"role": "assistant", "content": format!("ok {i}")}));
        }
        let mut req = json!({ "messages": messages });
        if let Value::Object(m) = extra {
            for (k, v) in m {
                req[k] = v;
            }
        }
        serde_json::to_vec(&req).unwrap()
    }

    async fn post_to(app: Router, uri: &str, body: Vec<u8>) -> Response {
        app.oneshot(
            axum::http::Request::builder()
                .method("POST")
                .uri(uri)
                .header("content-type", "application/json")
                .body(Body::from(body))
                .expect("request is valid"),
        )
        .await
        .expect("oneshot should succeed")
    }

    #[tokio::test]
    async fn openai_proxy_compresses_and_forwards() {
        let mock = MockServer::start().await;
        Mock::given(method("POST"))
            .respond_with(ResponseTemplate::new(200).set_body_string("ok"))
            .expect(1)
            .mount(&mock)
            .await;

        let body = compressible_body(json!({"model": "gpt-4"}));
        let response = post_to(app(mock.uri()), "/v1/chat/completions", body.clone()).await;
        assert_eq!(response.status(), StatusCode::OK);

        let requests = mock.received_requests().await.expect("recording enabled");
        let forwarded = &requests.first().expect("one request").body;
        assert!(
            forwarded.len() < body.len(),
            "forwarded body ({}) should be smaller than original ({})",
            forwarded.len(),
            body.len()
        );
    }

    #[tokio::test]
    async fn anthropic_proxy_returns_upstream_status() {
        let mock = MockServer::start().await;
        Mock::given(method("POST"))
            .respond_with(ResponseTemplate::new(201).set_body_string("created"))
            .expect(1)
            .mount(&mock)
            .await;

        let body = compressible_body(json!({"model": "claude-3", "max_tokens": 100}));
        let response = post_to(app(mock.uri()), "/v1/messages", body).await;
        assert_eq!(response.status(), StatusCode::CREATED);
    }

    #[tokio::test]
    async fn fails_open_with_502_when_upstream_is_down() {
        // Unreachable upstream port → reqwest error → 502 (never 500/panic).
        let body = br#"{"model":"gpt-4","messages":[{"role":"user","content":"hi"}]}"#.to_vec();
        let response = post_to(app("http://127.0.0.1:1".to_string()), "/v1/chat/completions", body).await;
        assert_eq!(response.status(), StatusCode::BAD_GATEWAY);
    }

    #[tokio::test]
    async fn forwards_client_authorization_header() {
        let mock = MockServer::start().await;
        // The mock ONLY matches when the Authorization header is present upstream,
        // so a 200 proves the proxy forwarded the client's auth (not dropped it).
        Mock::given(method("POST"))
            .and(wiremock::matchers::header("authorization", "Bearer secret-key"))
            .respond_with(ResponseTemplate::new(200).set_body_string("ok"))
            .expect(1)
            .mount(&mock)
            .await;

        let body = compressible_body(json!({"model": "gpt-4"}));
        let response = app(mock.uri())
            .oneshot(
                axum::http::Request::builder()
                    .method("POST")
                    .uri("/v1/chat/completions")
                    .header("content-type", "application/json")
                    .header("authorization", "Bearer secret-key")
                    .body(Body::from(body))
                    .expect("request is valid"),
            )
            .await
            .expect("oneshot should succeed");
        assert_eq!(response.status(), StatusCode::OK);
    }
}
