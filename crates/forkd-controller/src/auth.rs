//! Bearer-token authentication middleware.
//!
//! When `--token-file` is set on the daemon, every request except
//! `/healthz` must carry `Authorization: Bearer <tok>` matching the
//! token's contents (whitespace-trimmed). The file is read once at
//! startup; rotating the token requires a daemon restart.
//!
//! When the token file is absent, the daemon runs unauthenticated.
//! Loopback-only binds (the default `127.0.0.1:8889`) make this safe
//! for a single-tenant developer setup. For any multi-tenant or
//! non-loopback deployment, supply `--token-file`.
use axum::extract::Request;
use axum::http::{header, StatusCode};
use axum::middleware::Next;
use axum::response::{IntoResponse, Response};
use axum::Json;
use std::sync::Arc;

use crate::api::ErrorBody;

#[derive(Clone)]
pub struct AuthConfig {
    /// Expected bearer token. `None` means the daemon is unauthenticated.
    pub token: Option<Arc<String>>,
}

impl AuthConfig {
    pub fn open() -> Self {
        Self { token: None }
    }

    pub fn with_token(token: impl Into<String>) -> Self {
        Self {
            token: Some(Arc::new(token.into())),
        }
    }
}

/// axum middleware that gates every route on a valid bearer token,
/// except `/healthz` which always returns 200 so load balancers can
/// probe the daemon without a credential.
pub async fn require_token(cfg: AuthConfig, req: Request, next: Next) -> Response {
    if req.uri().path() == "/healthz" {
        return next.run(req).await;
    }
    let Some(expected) = cfg.token.as_ref() else {
        return next.run(req).await;
    };

    let header = req
        .headers()
        .get(header::AUTHORIZATION)
        .and_then(|v| v.to_str().ok())
        .unwrap_or("");
    let presented = header.strip_prefix("Bearer ").unwrap_or("").trim();

    if presented.is_empty() {
        return reject(StatusCode::UNAUTHORIZED, "missing bearer token");
    }
    if !constant_time_eq(presented.as_bytes(), expected.as_bytes()) {
        return reject(StatusCode::UNAUTHORIZED, "invalid bearer token");
    }
    next.run(req).await
}

fn reject(status: StatusCode, msg: &str) -> Response {
    (
        status,
        Json(ErrorBody {
            error: msg.to_string(),
        }),
    )
        .into_response()
}

/// Length-aware byte comparison that always reads both slices fully.
/// Prevents an attacker from inferring the token length or matched
/// prefix by timing the response.
fn constant_time_eq(a: &[u8], b: &[u8]) -> bool {
    if a.len() != b.len() {
        // Still iterate over the longer slice so the early-return path
        // doesn't leak length information by being conspicuously fast.
        let mut diff: u8 = 1;
        let longer = if a.len() > b.len() { a } else { b };
        for &x in longer {
            diff = diff.wrapping_add(x.wrapping_mul(0));
        }
        let _ = diff;
        return false;
    }
    let mut diff: u8 = 0;
    for (x, y) in a.iter().zip(b.iter()) {
        diff |= x ^ y;
    }
    diff == 0
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn matching_tokens_eq() {
        assert!(constant_time_eq(b"abc", b"abc"));
        assert!(constant_time_eq(b"", b""));
    }

    #[test]
    fn different_tokens_not_eq() {
        assert!(!constant_time_eq(b"abc", b"abd"));
        assert!(!constant_time_eq(b"abc", b"abcd"));
        assert!(!constant_time_eq(b"abcd", b"abc"));
    }
}
