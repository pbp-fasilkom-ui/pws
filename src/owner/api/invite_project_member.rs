use axum::extract::{Path, State};
use axum::response::Response;
use axum::Json;
use hyper::{Body, StatusCode};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

use crate::{auth::Auth, startup::AppState};
use sqlx::Row;

#[derive(Deserialize, Debug)]
pub struct ShareRequest {
    pub username: String,
}

#[derive(Serialize, Debug)]
struct ErrorResponse {
    message: String,
}

#[tracing::instrument(skip(auth, pool))]
pub async fn post(
    auth: Auth,
    State(AppState { pool, .. }): State<AppState>,
    Path((owner, project)): Path<(String, String)>,
    Json(req): Json<ShareRequest>,
) -> Response<Body> {
    let Some(user) = auth.current_user else {
        let json = serde_json::to_string(&ErrorResponse {
            message: "Unauthorized".to_string(),
        })
        .unwrap();
        return Response::builder()
            .status(StatusCode::UNAUTHORIZED)
            .header(axum::http::header::CONTENT_TYPE, "application/json")
            .body(Body::from(json))
            .unwrap();
    };

    // Get project ID
    let project_record = sqlx::query(
        r#"SELECT projects.id FROM projects
           JOIN project_owners ON projects.owner_id = project_owners.id
           WHERE projects.name = $1 AND project_owners.name = $2"#,
    )
    .bind(&project)
    .bind(&owner)
    .fetch_optional(&pool)
    .await
    .unwrap();

    let Some(record) = project_record else {
        let json = serde_json::to_string(&ErrorResponse {
            message: "Project not found".to_string(),
        })
        .unwrap();
        return Response::builder()
            .status(StatusCode::NOT_FOUND)
            .header(axum::http::header::CONTENT_TYPE, "application/json")
            .body(Body::from(json))
            .unwrap();
    };

    let project_id: Uuid = record.get("id");

    // Get target user
    let target_user = sqlx::query(r#"SELECT id FROM users WHERE username = $1"#)
        .bind(&req.username)
        .fetch_optional(&pool)
        .await
        .unwrap();

    let Some(user_record) = target_user else {
        let json = serde_json::to_string(&ErrorResponse {
            message: "User not found".to_string(),
        })
        .unwrap();
        return Response::builder()
            .status(StatusCode::BAD_REQUEST)
            .header(axum::http::header::CONTENT_TYPE, "application/json")
            .body(Body::from(json))
            .unwrap();
    };

    let target_user_id: Uuid = user_record.get("id");

    // Share project
    sqlx::query(
        r#"INSERT INTO project_shares (project_id, user_id) VALUES ($1, $2)
           ON CONFLICT DO NOTHING"#,
    )
    .bind(project_id)
    .bind(target_user_id)
    .execute(&pool)
    .await
    .unwrap();

    Response::builder()
        .status(StatusCode::OK)
        .header(axum::http::header::CONTENT_TYPE, "application/json")
        .body(Body::from(r#"{"message": "Project shared successfully"}"#))
        .unwrap()
}
