use axum::body::Body;
use axum::{
    middleware,
    routing::{get, post},
    Router,
};
use axum_extra::routing::RouterExt;

use crate::rate_limit::{rate_limit, RateLimiter};
use crate::{configuration::Settings, startup::AppState};

mod login;
mod logout;
mod register;
mod sso;
mod validate;

pub async fn router(_state: AppState, _config: &Settings) -> Router<AppState> {
    // Endpoints that accept credentials or a ticket. Without a limit here,
    // online guessing against /api/login was free.
    let credential_routes = Router::new()
        .route_with_tsr("/api/register", post(register::register_user))
        .route_with_tsr("/api/login", post(login::login_user))
        .route_with_tsr("/api/sso-callback", post(sso::handle_callback))
        .route_layer(middleware::from_fn_with_state(
            RateLimiter::new(),
            rate_limit,
        ));

    Router::new()
        .merge(credential_routes)
        // POST only: as a GET this could be triggered by a top-level
        // navigation from any page, logging the user out.
        .route_with_tsr("/api/logout", post(logout::logout_user))
        .route_with_tsr("/api/validate", get(validate::validate_auth))
}
