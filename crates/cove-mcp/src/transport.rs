use std::{net::SocketAddr, sync::Arc};

use axum::{
    body::Body,
    extract::{Request, State},
    http::{header, HeaderMap, StatusCode},
    middleware::{self, Next},
    response::Response,
    routing::any_service,
    Router,
};
use rmcp::{
    transport::{
        stdio,
        streamable_http_server::{
            session::local::LocalSessionManager, StreamableHttpServerConfig, StreamableHttpService,
        },
    },
    ServiceExt,
};
use tokio_util::sync::CancellationToken;

use crate::{
    config::{AuthConfig, CoveMcpConfig, CoveMcpError, HttpConfig},
    server::CoveMcpServer,
};

pub async fn serve_stdio(config: Arc<CoveMcpConfig>) -> Result<(), CoveMcpError> {
    let service = CoveMcpServer::new(config)
        .serve(stdio())
        .await
        .map_err(|error| CoveMcpError::Config(error.to_string()))?;
    service
        .waiting()
        .await
        .map_err(|error| CoveMcpError::Config(error.to_string()))?;
    Ok(())
}

pub async fn serve_http(config: Arc<CoveMcpConfig>, http: HttpConfig) -> Result<(), CoveMcpError> {
    http.validate()?;
    let token = CancellationToken::new();
    let allowed_hosts = allowed_hosts_for_bind(http.bind);
    let rmcp_config = StreamableHttpServerConfig::default()
        .with_stateful_mode(true)
        .with_sse_keep_alive(None)
        .with_allowed_hosts(allowed_hosts)
        .with_allowed_origins(http.allowed_origins.clone())
        .with_cancellation_token(token.clone());
    let session_manager = Arc::new(LocalSessionManager::default());
    let service: StreamableHttpService<CoveMcpServer, LocalSessionManager> =
        StreamableHttpService::new(
            move || Ok(CoveMcpServer::new(config.clone())),
            session_manager,
            rmcp_config,
        );
    let state = HttpMiddlewareState {
        auth: http.auth,
        allowed_origins: http.allowed_origins,
    };
    let app = Router::new()
        .route_service("/mcp", any_service(service))
        .layer(middleware::from_fn_with_state(state, http_guard));
    let listener = tokio::net::TcpListener::bind(http.bind)
        .await
        .map_err(CoveMcpError::Io)?;
    tracing::info!("cove-mcp HTTP listening on http://{}/mcp", http.bind);
    axum::serve(listener, app)
        .with_graceful_shutdown(shutdown_signal(token))
        .await
        .map_err(CoveMcpError::Io)?;
    Ok(())
}

async fn shutdown_signal(token: CancellationToken) {
    let _ = tokio::signal::ctrl_c().await;
    token.cancel();
}

#[derive(Clone)]
struct HttpMiddlewareState {
    auth: AuthConfig,
    allowed_origins: Vec<String>,
}

async fn http_guard(
    State(state): State<HttpMiddlewareState>,
    request: Request<Body>,
    next: Next,
) -> Result<Response, StatusCode> {
    validate_origin(request.headers(), &state.allowed_origins)?;
    validate_auth(request.headers(), &state.auth)?;
    Ok(next.run(request).await)
}

fn validate_auth(headers: &HeaderMap, auth: &AuthConfig) -> Result<(), StatusCode> {
    let Some(token) = auth.required_token() else {
        if auth.allow_no_auth_local {
            return Ok(());
        }
        return Err(StatusCode::UNAUTHORIZED);
    };
    let expected = format!("Bearer {token}");
    if headers
        .get(header::AUTHORIZATION)
        .and_then(|value| value.to_str().ok())
        == Some(expected.as_str())
    {
        Ok(())
    } else {
        Err(StatusCode::UNAUTHORIZED)
    }
}

fn validate_origin(headers: &HeaderMap, allowed_origins: &[String]) -> Result<(), StatusCode> {
    let Some(origin) = headers
        .get(header::ORIGIN)
        .and_then(|value| value.to_str().ok())
    else {
        return Ok(());
    };
    if origin.starts_with("http://127.0.0.1")
        || origin.starts_with("http://localhost")
        || origin.starts_with("http://[::1]")
        || allowed_origins.iter().any(|allowed| allowed == origin)
    {
        return Ok(());
    }
    Err(StatusCode::FORBIDDEN)
}

fn allowed_hosts_for_bind(bind: SocketAddr) -> Vec<String> {
    let mut hosts = vec!["localhost".into(), "127.0.0.1".into(), "::1".into()];
    let host = bind.ip().to_string();
    if !hosts.iter().any(|allowed| allowed == &host) {
        hosts.push(host.clone());
    }
    hosts.push(format!("{host}:{}", bind.port()));
    hosts
}

pub fn default_http_bind() -> SocketAddr {
    SocketAddr::from(([127, 0, 0, 1], 8765))
}

#[cfg(test)]
mod tests {
    use super::*;
    use axum::http::HeaderValue;

    #[test]
    fn token_auth_requires_bearer() {
        let auth = AuthConfig {
            bearer_token: Some("secret".into()),
            allow_no_auth_local: false,
        };
        let mut headers = HeaderMap::new();
        assert_eq!(
            validate_auth(&headers, &auth),
            Err(StatusCode::UNAUTHORIZED)
        );
        headers.insert(
            header::AUTHORIZATION,
            HeaderValue::from_static("Bearer secret"),
        );
        assert_eq!(validate_auth(&headers, &auth), Ok(()));
    }

    #[test]
    fn configured_origin_is_allowed() {
        let mut headers = HeaderMap::new();
        headers.insert(
            header::ORIGIN,
            HeaderValue::from_static("https://example.test"),
        );
        assert_eq!(
            validate_origin(&headers, &["https://example.test".to_string()]),
            Ok(())
        );
        assert_eq!(validate_origin(&headers, &[]), Err(StatusCode::FORBIDDEN));
    }
}
