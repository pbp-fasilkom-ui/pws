use crate::{auth::Auth, startup::AppState};
use axum::extract::State;
use axum::response::Response;
use hyper::{Body, StatusCode};
use serde::Serialize;
use uuid::Uuid;
use sqlx::Row;

#[derive(Serialize, Debug)]
struct Project {
    id: Uuid,
    name: String,
    owner_name: String,
}

#[derive(Serialize, Debug)]
struct DashboardProjectResponse {
    data: Vec<Project>
}
pub async fn get(auth: Auth, State(AppState { pool, .. }): State<AppState>) -> Response<Body> {
    let Some(user) = auth.current_user else {
        return Response::builder()
            .status(StatusCode::UNAUTHORIZED)
            .header(axum::http::header::CONTENT_TYPE, "application/json")
            .body(Body::from(r#"{"message": "Unauthorized"}"#))
            .unwrap();
    };

    // Get projects user owns OR is shared with
    let projects_result = sqlx::query(
        r#"SELECT DISTINCT projects.id, projects.name AS project, project_owners.name AS owner
           FROM projects
           JOIN project_owners ON projects.owner_id = project_owners.id
           LEFT JOIN users_owners ON project_owners.id = users_owners.owner_id
           LEFT JOIN project_shares ON projects.id = project_shares.project_id
           WHERE users_owners.user_id = $1 OR project_shares.user_id = $1
           ORDER BY projects.name ASC
        "#,
    )
    .bind(user.id)
    .fetch_all(&pool)
    .await;

    let projects_data = match projects_result {
        Ok(data) => data,
        Err(err) => {
            tracing::error!(?err, "Can't get projects: Failed to query database");
            return Response::builder()
                .status(StatusCode::INTERNAL_SERVER_ERROR)
                .header(axum::http::header::CONTENT_TYPE, "application/json")
                .body(Body::from(r#"{"message": "Failed to query database"}"#))
                .unwrap();
        }
    };

    let projects: Vec<Project> = projects_data.into_iter().map(|record| {
        Project {
            id: record.get::<Uuid, _>("id"),
            name: record.get::<String, _>("project"),
            owner_name: record.get::<String, _>("owner"),
        }
    }).collect();

    let json = serde_json::to_string(&DashboardProjectResponse {
        data: projects
    }).unwrap();

    Response::builder()
        .status(StatusCode::OK)
        .header(axum::http::header::CONTENT_TYPE, "application/json")
        .body(Body::from(json))
        .unwrap()
} 
