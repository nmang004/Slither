use axum::extract::Request;
use axum::middleware::Next;
use axum::response::Response;
use sha2::{Digest, Sha256};

use crate::error::ApiError;

/// Constant-time comparison via SHA-256: both sides are hashed to fixed-length
/// digests so the `==` on `[u8; 32]` cannot leak token length or content via
/// timing.
fn constant_time_eq(a: &str, b: &str) -> bool {
    let hash_a = Sha256::digest(a.as_bytes());
    let hash_b = Sha256::digest(b.as_bytes());
    hash_a == hash_b // Fixed-length comparison, no short-circuit on content
}

/// Log a one-time warning at startup when no API key is configured.
/// Called during server initialization.
pub fn warn_if_unauthenticated() {
    if crate::API_KEY.get().is_none() {
        tracing::warn!(
            "No SLITHER_API_KEY set — API is unauthenticated. \
             Set SLITHER_API_KEY for production use."
        );
    }
}

/// Extract the credential from an `Authorization: Bearer <token>` header.
///
/// RFC 7235 defines the auth-scheme as case-insensitive, so `bearer`, `Bearer`
/// and `BEARER` are all valid; matching only the exact `"Bearer "` prefix
/// rejected otherwise-correct clients.
fn bearer_token(header: &str) -> Option<&str> {
    let (scheme, token) = header.split_once(' ')?;
    if scheme.eq_ignore_ascii_case("bearer") {
        Some(token.trim_start())
    } else {
        None
    }
}

/// Axum middleware that optionally enforces bearer-token auth.
///
/// - If no API key is configured, every request passes through.
/// - If one **is** configured, the request must carry `Authorization: Bearer <key>`.
pub async fn auth_middleware(request: Request, next: Next) -> Result<Response, ApiError> {
    if let Some(expected) = crate::API_KEY.get() {
        let header = request
            .headers()
            .get("authorization")
            .and_then(|v| v.to_str().ok());

        match header.and_then(bearer_token) {
            Some(token) => {
                if !constant_time_eq(token, expected) {
                    return Err(ApiError::unauthorized("Invalid API key"));
                }
            }
            _ => {
                return Err(ApiError::unauthorized(
                    "Missing or malformed Authorization header",
                ));
            }
        }
    }

    Ok(next.run(request).await)
}

#[cfg(test)]
mod tests {
    use super::bearer_token;
    use axum::body::Body;
    use axum::http::{Request, StatusCode};
    use axum::routing::get;
    use axum::Router;
    use tower::ServiceExt;

    /// Auth enforcement lives in one test because the API key is a process-wide
    /// `OnceLock`: it can be set exactly once, so the configured and
    /// unconfigured cases cannot both be exercised in the same binary.
    #[tokio::test]
    async fn a_configured_key_is_required_and_must_match() {
        crate::API_KEY
            .set("s3cret".to_string())
            .expect("no other test may set the key");

        let app = Router::new()
            .route("/guarded", get(|| async { "ok" }))
            .layer(axum::middleware::from_fn(super::auth_middleware));

        let request = |header: Option<&str>| {
            let mut builder = Request::builder().uri("/guarded");
            if let Some(value) = header {
                builder = builder.header("authorization", value);
            }
            builder.body(Body::empty()).unwrap()
        };

        let missing = app.clone().oneshot(request(None)).await.unwrap();
        assert_eq!(missing.status(), StatusCode::UNAUTHORIZED);

        let wrong = app
            .clone()
            .oneshot(request(Some("Bearer wrong")))
            .await
            .unwrap();
        assert_eq!(wrong.status(), StatusCode::UNAUTHORIZED);

        let malformed = app
            .clone()
            .oneshot(request(Some("Basic s3cret")))
            .await
            .unwrap();
        assert_eq!(malformed.status(), StatusCode::UNAUTHORIZED);

        let correct = app
            .clone()
            .oneshot(request(Some("Bearer s3cret")))
            .await
            .unwrap();
        assert_eq!(correct.status(), StatusCode::OK);

        // The scheme is case-insensitive per RFC 7235.
        let lowercase = app.oneshot(request(Some("bearer s3cret"))).await.unwrap();
        assert_eq!(lowercase.status(), StatusCode::OK);
    }

    #[test]
    fn bearer_scheme_is_case_insensitive() {
        assert_eq!(bearer_token("Bearer secret"), Some("secret"));
        assert_eq!(bearer_token("bearer secret"), Some("secret"));
        assert_eq!(bearer_token("BEARER secret"), Some("secret"));
        assert_eq!(bearer_token("BeArEr secret"), Some("secret"));
    }

    #[test]
    fn other_schemes_and_malformed_headers_are_rejected() {
        assert_eq!(bearer_token("Basic secret"), None);
        assert_eq!(bearer_token("secret"), None);
        assert_eq!(bearer_token(""), None);
    }

    #[test]
    fn token_is_returned_verbatim() {
        // Only the separating space is consumed; the token keeps its own shape.
        assert_eq!(bearer_token("Bearer  padded"), Some("padded"));
        assert_eq!(bearer_token("Bearer a.b-c_d"), Some("a.b-c_d"));
    }
}
