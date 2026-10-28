use axum::body::Body;
use axum::extract::{Path, State};
use axum::http::StatusCode;
use axum::response::Response;
use uuid::Uuid;

use crate::{auth::Auth, authz, startup::AppState};
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

    // Owner-only, for the same reason as invite: otherwise any user could strip
    // collaborators from any project.
    match authz::is_project_owner(&pool, &owner, &project, user.id).await {
        Ok(true) => {}
        Ok(false) => {
            let json = serde_json::to_string(&ErrorResponse {
                message: "Project not found or you don't have access".to_string(),
            })
            .unwrap();
            return Response::builder()
                .status(StatusCode::NOT_FOUND)
                .header(axum::http::header::CONTENT_TYPE, "application/json")
                .body(Body::from(json))
                .unwrap();
        }
        Err(err) => {
            tracing::error!(?err, "Can't remove member: Failed to check ownership");
            let json = serde_json::to_string(&ErrorResponse {
                message: "Failed to check project ownership".to_string(),
            })
            .unwrap();
            return Response::builder()
                .status(StatusCode::INTERNAL_SERVER_ERROR)
                .header(axum::http::header::CONTENT_TYPE, "application/json")
                .body(Body::from(json))
                .unwrap();
        }
    }

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
