use axum::{
    Router,
    http::{StatusCode, Uri, header},
    response::{Html, IntoResponse, Response},
};
use rust_embed::Embed;

#[derive(Embed)]
#[folder = "frontend/dist"]
struct Assets;

pub fn router() -> Router {
    Router::new().fallback(serve)
}

async fn serve(uri: Uri) -> Response {
    let path = uri.path().trim_start_matches('/');

    if let Some(file) = Assets::get(path) {
        let mime = mime_guess::from_path(path).first_or_octet_stream();
        (
            StatusCode::OK,
            [(header::CONTENT_TYPE, mime.as_ref())],
            file.data,
        )
            .into_response()
    } else if let Some(index) = Assets::get("index.html") {
        Html(String::from_utf8_lossy(&index.data).into_owned()).into_response()
    } else {
        (StatusCode::NOT_FOUND, "not found").into_response()
    }
}
