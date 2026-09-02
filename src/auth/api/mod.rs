use axum::{
    routing::{get, post},
    Router,
};
use axum_extra::routing::RouterExt;
use hyper::Body;

use crate::{configuration::Settings, startup::AppState};

mod login;
mod logout;
mod register;
mod sso;
mod validate;

pub async fn router(_state: AppState, _config: &Settings) -> Router<AppState, Body> {
    Router::new()
        .route_with_tsr("/api/register", post(register::register_user))
        .route_with_tsr("/api/login", post(login::login_user))
        .route_with_tsr(
            "/api/logout",
            get(logout::logout_user).post(logout::logout_user),
        )
        .route_with_tsr("/api/validate", get(validate::validate_auth))
        .route_with_tsr("/api/sso-callback", post(sso::handle_callback))
}
