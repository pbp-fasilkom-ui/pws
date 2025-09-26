use axum::{middleware, Router, routing::{get, post}};
use axum_extra::routing::RouterExt;
use hyper::Body;

use crate::{auth::auth, startup::AppState, configuration::Settings};

mod create_project;
mod project_dashboard;
mod web_terminal;
mod delete_project;
mod delete_volume;
mod view_build_log;
mod view_container_log;
mod view_project_environ;
mod update_project_environ;
mod delete_project_environ;
mod bulk_update_project_environ;
mod generate_status_badge;
mod get_project_status;
mod get_git_credentials;
mod regenerate_git_password;
mod view_project_tree;
mod check_project_access;

pub async fn router(_state: AppState, _config: &Settings) -> Router<AppState, Body> {
    Router::new()
        .route_with_tsr("/api/project/new", post(create_project::post))
        .route_with_tsr("/api/project/:owner/:project/access", get(check_project_access::get))
        .route_with_tsr("/api/project/:owner/:project/builds", get(project_dashboard::get))
        .route_with_tsr("/api/project/:owner/:project/logs", get(view_container_log::get))
        .route_with_tsr("/api/project/:owner/:project/env", get(view_project_environ::get).post(update_project_environ::post))
        .route_with_tsr("/api/project/:owner/:project/env/bulk", post(bulk_update_project_environ::post))
        .route_with_tsr("/api/project/:owner/:project/env/delete", post(delete_project_environ::post))
        .route_with_tsr("/api/project/:owner/:project/builds/:build_id", get(view_build_log::get))
        .route_with_tsr("/api/project/:owner/:project/delete", post(delete_project::post))
        .route_with_tsr("/api/project/:owner/:project/volume/delete", post(delete_volume::post))
        .route_with_tsr("/api/project/:owner/:project/terminal/ws", get(web_terminal::ws))
        .route_with_tsr("/api/project/:owner/:project/git-credentials", get(get_git_credentials::get))
        .route_with_tsr("/api/project/:owner/:project/regenerate-git-password", post(regenerate_git_password::post))
        .route_with_tsr("/api/project/:owner/:project/tree", get(view_project_tree::get))
        .route_layer(middleware::from_fn(auth))
        .route_with_tsr("/api/project/:owner/:project/badge/status", get(generate_status_badge::get))
        .route_with_tsr("/api/project/:owner/:project/status", get(get_project_status::get))
}
