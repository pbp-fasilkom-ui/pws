use axum::extract::{State, Path};
use axum::response::Response;
use hyper::{Body, StatusCode};
use serde::Serialize;
use uuid::Uuid;

use crate::{auth::Auth, startup::AppState};
use sqlx::Row;

#[derive(Serialize, Debug)]
struct ProjectShare {
    user_id: Uuid,
    username: String,
    name: String,
    created_at: chrono::DateTime<chrono::Utc>,
}

#[derive(Serialize, Debug)]
struct ProjectSharesResponse {
    shares: Vec<ProjectShare>,
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
    let Some(_user) = auth.current_user else {
        let json = serde_json::to_string(&ErrorResponse {
            message: "Unauthorized".to_string(),
        }).unwrap();
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
        }).unwrap();
        return Response::builder()
            .status(StatusCode::NOT_FOUND)
            .header(axum::http::header::CONTENT_TYPE, "application/json")
            .body(Body::from(json))
            .unwrap();
    };

    let project_id: Uuid = record.get("id");

    // Get project shares
    let shares_result = sqlx::query(
        r#"SELECT u.id, u.username, u.name, ps.created_at
           FROM users u
           JOIN project_shares ps ON u.id = ps.user_id
           WHERE ps.project_id = $1
           ORDER BY ps.created_at ASC
        "#,
    )
    .bind(project_id)
    .fetch_all(&pool)
    .await
    .unwrap();

    let shares: Vec<ProjectShare> = shares_result
        .into_iter()
        .map(|row| ProjectShare {
            user_id: row.get::<Uuid, _>("id"),
            username: row.get::<String, _>("username"),
            name: row.get::<String, _>("name"),
            created_at: row.get::<chrono::DateTime<chrono::Utc>, _>("created_at"),
        })
        .collect();

    let json = serde_json::to_string(&ProjectSharesResponse { shares }).unwrap();

    Response::builder()
        .status(StatusCode::OK)
        .header(axum::http::header::CONTENT_TYPE, "application/json")
        .body(Body::from(json))
        .unwrap()
}