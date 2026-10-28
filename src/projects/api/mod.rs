use axum::body::Body;
use axum::{
    middleware,
    routing::{get, post},
    Router,
};
use axum_extra::routing::RouterExt;

use crate::{auth::auth, configuration::Settings, startup::AppState};

mod bulk_update_project_environ;
mod check_project_access;
mod create_project;
mod delete_project;
mod delete_project_environ;
mod delete_volume;
mod generate_status_badge;
mod get_git_credentials;
mod get_project_status;
mod project_dashboard;
mod redeploy_project;
mod regenerate_git_password;
mod stream_build_log;
mod update_project_environ;
mod view_build_log;
mod view_container_log;
mod view_project_environ;
mod view_project_file;
mod view_project_refs;
mod view_project_tree;
mod web_terminal;

pub async fn router(_state: AppState, _config: &Settings) -> Router<AppState> {
    Router::new()
        .route_with_tsr("/api/project/new", post(create_project::post))
        .route_with_tsr(
            "/api/project/{owner}/{project}/access",
            get(check_project_access::get),
        )
        .route_with_tsr(
            "/api/project/{owner}/{project}/builds",
            get(project_dashboard::get),
        )
        .route_with_tsr(
            "/api/project/{owner}/{project}/logs",
            get(view_container_log::get),
        )
        .route_with_tsr(
            "/api/project/{owner}/{project}/env",
            get(view_project_environ::get).post(update_project_environ::post),
        )
        .route_with_tsr(
            "/api/project/{owner}/{project}/env/bulk",
            post(bulk_update_project_environ::post),
        )
        .route_with_tsr(
            "/api/project/{owner}/{project}/env/delete",
            post(delete_project_environ::post),
        )
        .route_with_tsr(
            "/api/project/{owner}/{project}/builds/{build_id}",
            get(view_build_log::get),
        )
        .route_with_tsr(
            "/api/project/{owner}/{project}/builds/{build_id}/events",
            get(stream_build_log::get),
        )
        .route_with_tsr(
            "/api/project/{owner}/{project}/delete",
            post(delete_project::post),
        )
        .route_with_tsr(
            "/api/project/{owner}/{project}/volume/delete",
            post(delete_volume::post),
        )
        .route_with_tsr(
            "/api/project/{owner}/{project}/terminal/ws",
            get(web_terminal::ws),
        )
        .route_with_tsr(
            "/api/project/{owner}/{project}/git-credentials",
            get(get_git_credentials::get),
        )
        .route_with_tsr(
            "/api/project/{owner}/{project}/regenerate-git-password",
            post(regenerate_git_password::post),
        )
        .route_with_tsr(
            "/api/project/{owner}/{project}/redeploy",
            post(redeploy_project::post),
        )
        .route_with_tsr(
            "/api/project/{owner}/{project}/tree",
            get(view_project_tree::get),
        )
        .route_with_tsr(
            "/api/project/{owner}/{project}/refs",
            get(view_project_refs::get),
        )
        .route_with_tsr(
            "/api/project/{owner}/{project}/file",
            get(view_project_file::get),
        )
        .route_layer(middleware::from_fn(auth))
        .route_with_tsr(
            "/api/project/{owner}/{project}/badge/status",
            get(generate_status_badge::get),
        )
        .route_with_tsr(
            "/api/project/{owner}/{project}/status",
            get(get_project_status::get),
        )
}
