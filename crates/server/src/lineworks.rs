use axum::{
    Router,
    body::Bytes,
    extract::State,
    http::{HeaderMap, StatusCode},
    routing::post,
};
use base64::Engine;
use hmac::{Hmac, Mac};
use sha2::Sha256;
use std::sync::Arc;
use subtle::ConstantTimeEq;

type HmacSha256 = Hmac<Sha256>;

#[derive(Clone)]
pub struct LineworksAppState {
    bot_secret: Arc<[u8]>,
}

impl LineworksAppState {
    pub fn new(bot_secret: impl Into<Arc<[u8]>>) -> Self {
        Self {
            bot_secret: bot_secret.into(),
        }
    }
}

pub fn router(bot_secret: impl Into<Arc<[u8]>>) -> Router {
    Router::new()
        .route("/api/lineworks/callback", post(callback))
        .with_state(LineworksAppState::new(bot_secret))
}

pub fn verify_lineworks_signature(body: &[u8], signature: &str, secret: &[u8]) -> bool {
    let provided_signature = match base64::engine::general_purpose::STANDARD.decode(signature) {
        Ok(decoded) => decoded,
        Err(_) => return false,
    };

    let mut mac = match HmacSha256::new_from_slice(secret) {
        Ok(mac) => mac,
        Err(_) => return false,
    };

    mac.update(body);
    let expected_signature = mac.finalize().into_bytes();

    expected_signature.ct_eq(&provided_signature).into()
}

async fn callback(
    State(state): State<LineworksAppState>,
    headers: HeaderMap,
    body: Bytes,
) -> StatusCode {
    let Some(signature) = headers
        .get("X-WORKS-Signature")
        .and_then(|value| value.to_str().ok())
    else {
        return StatusCode::UNAUTHORIZED;
    };

    if verify_lineworks_signature(&body, signature, &state.bot_secret) {
        StatusCode::NO_CONTENT
    } else {
        StatusCode::UNAUTHORIZED
    }
}

#[cfg(test)]
mod tests {
    use super::{HmacSha256, router, verify_lineworks_signature};
    use axum::{body::Body, http::Request};
    use base64::Engine;
    use hmac::Mac;
    use tower::ServiceExt;

    #[tokio::test]
    // 正しい署名は検証に成功する。
    async fn accepts_valid_signature() {
        let body = br#"{"type":"message"}"#;
        let secret = b"secret";
        let signature = signature_for(body, secret);

        assert!(verify_lineworks_signature(body, &signature, secret));
    }

    #[tokio::test]
    // ボディが改ざんされると検証に失敗する。
    async fn rejects_tampered_body() {
        let body = br#"{"type":"message"}"#;
        let secret = b"secret";
        let signature = signature_for(body, secret);

        assert!(!verify_lineworks_signature(
            br#"{"type":"other"}"#,
            &signature,
            secret
        ));
    }

    #[tokio::test]
    // 署名文字列が Base64 でなければ検証に失敗する。
    async fn rejects_invalid_base64_signature() {
        let body = br#"{"type":"message"}"#;

        assert!(!verify_lineworks_signature(body, "not-base64", b"secret"));
    }

    #[tokio::test]
    // 別の secret で作られた署名は受け付けない。
    async fn rejects_signature_with_wrong_secret() {
        let body = br#"{"type":"message"}"#;
        let signature = signature_for(body, b"secret-a");

        assert!(!verify_lineworks_signature(body, &signature, b"secret-b"));
    }

    #[tokio::test]
    // callback は正しい署名があれば 204 を返す。
    async fn accepts_callback_with_valid_signature() {
        let body = br#"{"type":"message"}"#;
        let secret = b"secret";
        let signature = signature_for(body, secret);

        let request: Request<Body> = Request::builder()
            .method("POST")
            .uri("/api/lineworks/callback")
            .header("X-WORKS-Signature", signature)
            .body(Body::from(body.as_slice()))
            .expect("request should be built");

        let response = router(secret.to_vec())
            .oneshot(request)
            .await
            .expect("response should be returned");

        assert_eq!(response.status(), axum::http::StatusCode::NO_CONTENT);
    }

    #[tokio::test]
    // callback は署名が無ければ 401 を返す。
    async fn rejects_callback_without_signature() {
        let body = br#"{"type":"message"}"#;
        let secret = b"secret";

        let request: Request<Body> = Request::builder()
            .method("POST")
            .uri("/api/lineworks/callback")
            .body(Body::from(body.as_slice()))
            .expect("request should be built");

        let response = router(secret.to_vec())
            .oneshot(request)
            .await
            .expect("response should be returned");

        assert_eq!(response.status(), axum::http::StatusCode::UNAUTHORIZED);
    }

    #[tokio::test]
    // callback は署名不一致なら 401 を返す。
    async fn rejects_callback_with_invalid_signature() {
        let body = br#"{"type":"message"}"#;
        let secret = b"secret";
        let signature = signature_for(body, b"other-secret");

        let request: Request<Body> = Request::builder()
            .method("POST")
            .uri("/api/lineworks/callback")
            .header("X-WORKS-Signature", signature)
            .body(Body::from(body.as_slice()))
            .expect("request should be built");

        let response = router(secret.to_vec())
            .oneshot(request)
            .await
            .expect("response should be returned");

        assert_eq!(response.status(), axum::http::StatusCode::UNAUTHORIZED);
    }

    fn signature_for(body: &[u8], secret: &[u8]) -> String {
        let mut mac = HmacSha256::new_from_slice(secret).expect("secret should be valid");
        mac.update(body);
        base64::engine::general_purpose::STANDARD.encode(mac.finalize().into_bytes())
    }
}
