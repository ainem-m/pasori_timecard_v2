use axum::{
    body::Body,
    http::{Response, StatusCode, header},
    response::IntoResponse,
};
use rust_embed::RustEmbed;

#[derive(RustEmbed)]
#[folder = "../../web/admin/dist/"]
struct Assets;

pub async fn static_handler(uri: axum::http::Uri) -> impl IntoResponse {
    let path = uri.path().trim_start_matches('/');

    if path.is_empty() || path == "index.html" {
        return index_html().await;
    }

    match Assets::get(path) {
        Some(content) => {
            let mime = mime_guess::from_path(path).first_or_octet_stream();
            build_response(
                StatusCode::OK,
                Some(mime.as_ref()),
                Body::from(content.data),
            )
        }
        None => {
            // Fallback to index.html for SPA routing
            if !path.contains('.') {
                index_html().await
            } else {
                build_response(StatusCode::NOT_FOUND, None, Body::empty())
            }
        }
    }
}

async fn index_html() -> Response<Body> {
    match Assets::get("index.html") {
        Some(content) => {
            build_response(StatusCode::OK, Some("text/html"), Body::from(content.data))
        }
        None => build_response(
            StatusCode::NOT_FOUND,
            Some("text/plain; charset=utf-8"),
            Body::from("404 Not Found"),
        ),
    }
}

fn build_response(status: StatusCode, content_type: Option<&str>, body: Body) -> Response<Body> {
    let mut builder = Response::builder().status(status);
    if let Some(content_type) = content_type {
        builder = builder.header(header::CONTENT_TYPE, content_type);
    }

    match builder.body(body) {
        Ok(response) => response,
        Err(error) => {
            tracing::error!(%error, ?status, "failed to build static asset response");
            Response::builder()
                .status(StatusCode::INTERNAL_SERVER_ERROR)
                .body(Body::from("internal server error"))
                .unwrap_or_else(|fallback_error| {
                    tracing::error!(
                        %fallback_error,
                        "failed to build fallback static asset response"
                    );
                    Response::new(Body::empty())
                })
        }
    }
}

#[cfg(test)]
mod tests {
    use super::{build_response, static_handler};
    use axum::{
        body::Body,
        http::{StatusCode, Uri, header},
        response::IntoResponse,
    };

    #[tokio::test]
    // 拡張子つきの未知パスは 404 を返す。
    async fn returns_not_found_for_missing_asset_path() {
        let response = static_handler(Uri::from_static("/missing.js"))
            .await
            .into_response();

        assert_eq!(response.status(), StatusCode::NOT_FOUND);
    }

    #[tokio::test]
    // 拡張子なしの未知パスは SPA fallback として index.html を返す。
    async fn returns_index_html_for_spa_route() {
        let response = static_handler(Uri::from_static("/employees"))
            .await
            .into_response();

        assert_eq!(response.status(), StatusCode::OK);
        assert_eq!(
            response.headers().get(header::CONTENT_TYPE),
            Some(&header::HeaderValue::from_static("text/html"))
        );
    }

    #[test]
    // 不正な Content-Type が渡されても panic せず 500 を返す。
    fn returns_internal_server_error_when_response_builder_fails() {
        let response = build_response(
            StatusCode::OK,
            Some("text/plain\r\nx-invalid: header"),
            Body::from("broken"),
        );

        assert_eq!(response.status(), StatusCode::INTERNAL_SERVER_ERROR);
    }
}
