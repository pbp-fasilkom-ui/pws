use crate::configuration::Settings;
use crate::{auth::auth, startup::AppState};
use axum::routing::get;
use axum::{middleware, Router};
use axum_extra::routing::RouterExt;
use hyper::Body;

mod get_dashboard_projects;

pub async fn router(_state: AppState, _config: &Settings) -> Router<AppState, Body> {
    Router::new()
        .route_with_tsr("/api/dashboard/project", get(get_dashboard_projects::get))
        .route_layer(middleware::from_fn(auth))
}
