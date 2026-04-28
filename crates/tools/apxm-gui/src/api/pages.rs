//! Page-serving handlers.

use axum::response::Html;

/// GET / - serve the embedded SPA index.html.
pub async fn index_handler() -> Html<&'static str> {
    Html(include_str!("../frontend-dist/index.html"))
}

/// SPA fallback - serve index.html for all non-API routes.
pub async fn spa_fallback() -> Html<&'static str> {
    Html(include_str!("../frontend-dist/index.html"))
}
