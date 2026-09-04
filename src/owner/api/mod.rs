use axum::body::Body;
use axum::{
    middleware,
    routing::{get, post},
    Router,
};
use axum_extra::routing::RouterExt;

use crate::{auth::auth, configuration::Settings, startup::AppState};

mod create_project_owner;
mod get_project_members;
mod invite_project_member;
mod remove_project_member;
mod update_project_owner;

pub async fn router(_state: AppState, _config: &Settings) -> Router<AppState> {
    Router::new()
        .route_with_tsr("/api/owner", post(create_project_owner::post))
        .route_with_tsr("/api/owner/{owner_id}", post(update_project_owner::post))
        .route_with_tsr(
            "/api/owner/{owner}/{project}/invite",
            post(invite_project_member::post),
        )
        .route_with_tsr(
            "/api/owner/{owner}/{project}/members",
            get(get_project_members::get),
        )
        .route_with_tsr(
            "/api/owner/{owner}/{project}/remove/{user_id}",
            post(remove_project_member::post),
        )
        .route_layer(middleware::from_fn(auth))
}
