use axum::http::{header, StatusCode, Uri};
use axum::response::{IntoResponse, Response};
use rust_embed::RustEmbed;

#[derive(RustEmbed)]
#[folder = "frontend/dist/"]
struct FrontendAssets;

/// Axum fallback handler: serves the embedded Vue SPA. A real embedded
/// asset (e.g. `/assets/index-abc123.js`) is served by exact path with its
/// correct MIME type. Anything else — a client-side route like
/// `/workflows/<uuid>`, or a genuinely missing asset — falls back to
/// `index.html` so Vue Router's history-mode routing can take over. This
/// only ever runs for requests that didn't match any `/rest/*`,
/// `/webhook/*`, or `/health` route, since Axum only calls a router's
/// fallback after every other route fails to match.
pub async fn serve_frontend(uri: Uri) -> Response {
    let path = uri.path().trim_start_matches('/');
    serve_embedded(if path.is_empty() { "index.html" } else { path })
}

fn serve_embedded(path: &str) -> Response {
    match FrontendAssets::get(path) {
        Some(file) => ([(header::CONTENT_TYPE, file.metadata.mimetype().to_string())], file.data).into_response(),
        None => match FrontendAssets::get("index.html") {
            Some(file) => ([(header::CONTENT_TYPE, file.metadata.mimetype().to_string())], file.data).into_response(),
            None => (StatusCode::NOT_FOUND, "frontend not built").into_response(),
        },
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn index_html_is_embedded_and_is_real_html() {
        // If this fails, frontend/dist/ either doesn't exist or wasn't built
        // (Task 2's `npm run build` must run before this crate compiles) --
        // not a bug in this module's own logic.
        let file = FrontendAssets::get("index.html").expect("frontend/dist/index.html must exist (run `npm run build` in frontend/ first)");
        let body = String::from_utf8_lossy(&file.data);
        assert!(body.contains("<div id=\"app\">"));
    }

    #[test]
    fn unknown_path_falls_back_to_index_html() {
        let response = serve_embedded("some/client/route/that/is/not/a/real/file");
        assert_eq!(response.status(), StatusCode::OK);
    }
}
