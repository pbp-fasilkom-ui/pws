use crate::auth::Auth;
use axum::body::Body;
use axum::http::StatusCode;
use axum::response::Response;

#[tracing::instrument(skip(auth))]
pub async fn logout_user(auth: Auth) -> Response<Body> {
    auth.logout_user();
    Response::builder()
        .status(StatusCode::FOUND)
        .header("Location", "/api/login")
        .body(Body::empty())
        .unwrap()
}
