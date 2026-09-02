use axum::extract::{Path, State};
use axum::response::Response;
use hyper::{Body, StatusCode};
use uuid::Uuid;

use crate::{auth::Auth, startup::AppState};
use serde::Serialize;
use sqlx::Row;

#[derive(Serialize, Debug)]
struct ErrorResponse {
    message: String,
}

#[tracing::instrument(skip(auth, pool))]
pub async fn post(
    auth: Auth,
    State(AppState { pool, .. }): State<AppState>,
    Path((owner, project, user_id)): Path<(String, String, Uuid)>,
) -> Response<Body> {
    let Some(_user) = auth.current_user else {
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

    // Remove from project shares
    sqlx::query(
        r#"DELETE FROM project_shares
           WHERE project_id = $1 AND user_id = $2"#,
    )
    .bind(project_id)
    .bind(user_id)
    .execute(&pool)
    .await
    .unwrap();

    Response::builder()
        .status(StatusCode::OK)
        .header(axum::http::header::CONTENT_TYPE, "application/json")
        .body(Body::from(
            r#"{"message": "Project unshared successfully"}"#,
        ))
        .unwrap()
}
