use axum::extract::{State, Path};
use axum::response::Response;
use hyper::{Body, StatusCode};
use serde::{Serialize, Deserialize};
use chrono::{DateTime, Utc};
use uuid::Uuid;

use crate::{auth::Auth, startup::AppState};

#[derive(Serialize, Deserialize, Debug, sqlx::Type)]
#[sqlx(type_name = "build_state", rename_all = "lowercase")] 
pub enum BuildState {
    PENDING,
    BUILDING,
    SUCCESSFUL,
    FAILED
}

#[derive(Serialize, Debug)]
struct ProjectStatusResponse {
    project: String,
    owner: String,
    status: BuildState,
    build_id: Uuid,
    created_at: DateTime<Utc>,
    updated_at: DateTime<Utc>,
    finished_at: Option<DateTime<Utc>>,
}

#[derive(Serialize, Debug)]
struct ErrorResponse {
    error: String,
}

#[tracing::instrument(skip(_auth, pool))]
pub async fn get(
    _auth: Auth,
    State(AppState { pool, .. }): State<AppState>,
    Path((owner, project)): Path<(String, String)>,
) -> Response<Body> {
    // Check if project exists
    let project_record = match sqlx::query_as::<_, (Uuid,)>(
        r#"SELECT projects.id
           FROM projects
           JOIN project_owners ON projects.owner_id = project_owners.id
           JOIN users_owners ON project_owners.id = users_owners.owner_id
           WHERE projects.name = $1
           AND project_owners.name = $2"#,
    )
    .bind(&project)
    .bind(&owner)
    .fetch_optional(&pool)
    .await
    {
        Ok(Some(record)) => record,
        Ok(None) => {
            let json = serde_json::to_string(&ErrorResponse {
                error: "Project not found".to_string()
            }).unwrap();

            return Response::builder()
                .status(StatusCode::NOT_FOUND)
                .header("Content-Type", "application/json")
                .body(Body::from(json))
                .unwrap();
        }
        Err(err) => {
            tracing::error!(?err, "Failed to query project");
            let json = serde_json::to_string(&ErrorResponse {
                error: "Database error".to_string()
            }).unwrap();

            return Response::builder()
                .status(StatusCode::INTERNAL_SERVER_ERROR)
                .header("Content-Type", "application/json")
                .body(Body::from(json))
                .unwrap();
        }
    };

    // Get latest build status
    let build = match sqlx::query_as::<_, (Uuid, Uuid, BuildState, DateTime<Utc>, DateTime<Utc>, Option<DateTime<Utc>>)>(
        r#"SELECT id, project_id, status, created_at, updated_at, finished_at
        FROM builds WHERE project_id = $1
        ORDER BY created_at DESC
        LIMIT 1"#,
    )
    .bind(project_record.0)
    .fetch_one(&pool)
    .await 
    {
        Ok(record) => record,
        Err(err) => {
            tracing::error!(?err, "Failed to query build status");
            let json = serde_json::to_string(&ErrorResponse {
                error: "Failed to get build status".to_string()
            }).unwrap();

            return Response::builder()
                .status(StatusCode::INTERNAL_SERVER_ERROR)
                .header("Content-Type", "application/json")
                .body(Body::from(json))
                .unwrap();
        }
    };

    let response = ProjectStatusResponse {
        project: project.clone(),
        owner: owner.clone(),
        status: build.2,
        build_id: build.0,
        created_at: build.3,
        updated_at: build.4,
        finished_at: build.5,
    };

    let json = serde_json::to_string(&response).unwrap();

    Response::builder()
        .status(StatusCode::OK)
        .header("Content-Type", "application/json")
        .header("Cache-Control", "no-cache")
        .body(Body::from(json))
        .unwrap()
}
