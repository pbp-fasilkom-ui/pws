use axum::extract::{State, Path};
use axum::response::Response;
use hyper::{Body, StatusCode};
use serde::Serialize;

use crate::{auth::Auth, startup::AppState};

#[derive(Serialize, Debug)]
struct AccessResponse {
    has_access: bool,
}

#[derive(Serialize, Debug)]
struct ErrorResponse {
    message: String,
}

#[tracing::instrument(skip(auth, pool))]
pub async fn get(
    auth: Auth,
    State(AppState { pool, .. }): State<AppState>,
    Path((owner, project)): Path<(String, String)>,
) -> Response<Body> {
    let Some(user) = auth.current_user else {
        let json = serde_json::to_string(&ErrorResponse {
            message: "Unauthorized".to_string(),
        }).unwrap();
        return Response::builder()
            .status(StatusCode::UNAUTHORIZED)
            .header(axum::http::header::CONTENT_TYPE, "application/json")
            .body(Body::from(json))
            .unwrap();
    };

    // Check if user has access to this project (either as owner or shared)
    let has_access = sqlx::query(
        r#"SELECT 1 FROM projects
           JOIN project_owners ON projects.owner_id = project_owners.id
           LEFT JOIN users_owners ON project_owners.id = users_owners.owner_id
           LEFT JOIN project_shares ON projects.id = project_shares.project_id
           WHERE projects.name = $1
             AND project_owners.name = $2
             AND (users_owners.user_id = $3 OR project_shares.user_id = $3)
        "#,
    )
    .bind(&project)
    .bind(&owner)
    .bind(user.id)
    .fetch_optional(&pool)
    .await
    .map(|result| result.is_some())
    .unwrap_or(false);

    if !has_access {
        let json = serde_json::to_string(&ErrorResponse {
            message: "Project not found or you don't have access".to_string(),
        }).unwrap();
        return Response::builder()
            .status(StatusCode::NOT_FOUND)
            .header(axum::http::header::CONTENT_TYPE, "application/json")
            .body(Body::from(json))
            .unwrap();
    }

    let json = serde_json::to_string(&AccessResponse {
        has_access: true,
    }).unwrap();

    Response::builder()
        .status(StatusCode::OK)
        .header(axum::http::header::CONTENT_TYPE, "application/json")
        .body(Body::from(json))
        .unwrap()
}