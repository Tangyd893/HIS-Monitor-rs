//! API 中间件链
//!
//! 按请求处理顺序：
//!   Request → Tracing → CORS → RateLimit → Auth → Logger → Handler → Response

use axum::{
    extract::Request,
    http::{Method, StatusCode},
    middleware::Next,
    response::Response,
};
use std::time::Instant;
use tracing::{info, info_span};

/// 请求追踪 Span 中间件
///
/// 为每个 HTTP 请求创建 OpenTelemetry-compatible span，
/// 记录 method、uri、status、latency。
pub async fn tracing_middleware(req: Request, next: Next) -> Response {
    let method = req.method().clone();
    let uri = req.uri().path().to_string();

    let span = info_span!(
        "http_request",
        http.method = %method,
        http.uri = %uri,
    );

    let _enter = span.enter();
    let start = Instant::now();
    let response = next.run(req).await;
    let latency = start.elapsed();

    info!(
        method = %method,
        uri = %uri,
        status = response.status().as_u16(),
        latency_ms = latency.as_millis() as u64,
        "request completed"
    );

    response
}

/// CORS 中间件
///
/// 允许 Grafana IFrame 嵌入和运营管理后台 API 调用。
pub fn cors_layer() -> tower_http::cors::CorsLayer {
    tower_http::cors::CorsLayer::new()
        .allow_origin(tower_http::cors::Any)
        .allow_methods([Method::GET, Method::POST, Method::PUT, Method::DELETE])
        .allow_headers(tower_http::cors::Any)
}

/// 简单令牌桶限流中间件
///
/// 基于固定窗口 + 计数器实现，限制每个 IP 的 QPS。
#[derive(Clone)]
pub struct RateLimitLayer {
    /// 每个窗口允许的最大请求数
    max_requests: u32,
    /// 窗口大小（秒）
    window_secs: u64,
}

impl RateLimitLayer {
    pub fn new(max_requests: u32, window_secs: u64) -> Self {
        Self {
            max_requests,
            window_secs,
        }
    }
}

impl<S> tower::Layer<S> for RateLimitLayer {
    type Service = RateLimitMiddleware<S>;

    fn layer(&self, inner: S) -> Self::Service {
        RateLimitMiddleware {
            inner,
            max_requests: self.max_requests,
            window_secs: self.window_secs,
            buckets: std::sync::Arc::new(tokio::sync::Mutex::new(
                std::collections::HashMap::new(),
            )),
        }
    }
}

#[derive(Clone)]
pub struct RateLimitMiddleware<S> {
    inner: S,
    max_requests: u32,
    window_secs: u64,
    buckets: std::sync::Arc<
        tokio::sync::Mutex<std::collections::HashMap<String, (u32, std::time::Instant)>>,
    >,
}

impl<S, B> tower::Service<Request<B>> for RateLimitMiddleware<S>
where
    S: tower::Service<Request<B>, Response = Response> + Clone + Send + 'static,
    S::Future: Send,
    B: Send + 'static,
{
    type Response = S::Response;
    type Error = S::Error;
    type Future = std::pin::Pin<
        Box<dyn std::future::Future<Output = Result<Self::Response, Self::Error>> + Send>,
    >;

    fn poll_ready(
        &mut self,
        cx: &mut std::task::Context<'_>,
    ) -> std::task::Poll<Result<(), Self::Error>> {
        self.inner.poll_ready(cx)
    }

    fn call(&mut self, req: Request<B>) -> Self::Future {
        let client_ip = req
            .headers()
            .get("x-forwarded-for")
            .and_then(|v| v.to_str().ok())
            .unwrap_or("unknown")
            .to_string();

        let max = self.max_requests;
        let window = self.window_secs;
        let buckets = self.buckets.clone();
        let clone = self.inner.clone();
        let mut inner = std::mem::replace(&mut self.inner, clone);

        Box::pin(async move {
            let allowed = {
                let mut guard = buckets.lock().await;
                let now = std::time::Instant::now();
                let entry = guard.entry(client_ip.clone()).or_insert((0, now));

                if now.duration_since(entry.1).as_secs() >= window {
                    // 窗口重置
                    *entry = (1, now);
                    true
                } else if entry.0 < max {
                    entry.0 += 1;
                    true
                } else {
                    false
                }
            };

            if allowed {
                inner.call(req).await
            } else {
                let resp = Response::builder()
                    .status(StatusCode::TOO_MANY_REQUESTS)
                    .body(axum::body::Body::from("rate limit exceeded"))
                    .unwrap();
                Ok(resp)
            }
        })
    }
}

/// 请求日志中间件
pub async fn logger_middleware(req: Request, next: Next) -> Response {
    let method = req.method().clone();
    let uri = req.uri().clone();
    let user_agent = req
        .headers()
        .get("user-agent")
        .and_then(|v| v.to_str().ok())
        .unwrap_or("-")
        .to_string();

    info!(%method, %uri, %user_agent, "request started");
    next.run(req).await
}

// ── 测试 ──

#[cfg(test)]
mod tests {
    use super::*;
    use axum::{body::Body, routing::get, Router};
    use tower::ServiceExt;

    #[tokio::test]
    async fn test_tracing_middleware_does_not_block() {
        let app = Router::new()
            .route("/test", get(|| async { "ok" }))
            .layer(axum::middleware::from_fn(tracing_middleware));

        let req = Request::builder()
            .uri("/test")
            .body(Body::empty())
            .unwrap();
        let resp = app.oneshot(req).await.unwrap();
        assert_eq!(resp.status(), StatusCode::OK);
    }

    #[tokio::test]
    async fn test_logger_middleware() {
        let app = Router::new()
            .route("/log", get(|| async { "logged" }))
            .layer(axum::middleware::from_fn(logger_middleware));

        let req = Request::builder()
            .uri("/log")
            .header("user-agent", "test-agent/1.0")
            .body(Body::empty())
            .unwrap();

        let resp = app.oneshot(req).await.unwrap();
        assert_eq!(resp.status(), StatusCode::OK);
    }
}
