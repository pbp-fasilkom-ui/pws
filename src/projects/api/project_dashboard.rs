use std::fmt;

use axum::extract::{Path, State};
use axum::response::Response;
use chrono::{DateTime, Utc};
use hyper::{Body, StatusCode};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

use crate::{auth::Auth, authz, startup::AppState};

#[derive(Serialize, Deserialize, Debug, sqlx::Type)]
#[sqlx(type_name = "build_state", rename_all = "lowercase")]
pub enum BuildState {
    PENDING,
    BUILDING,
    SUCCESSFUL,
    FAILED,
}

impl fmt::Display for BuildState {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        match self {
            BuildState::PENDING => write!(f, "Pending"),
            BuildState::BUILDING => write!(f, "Building"),
            BuildState::SUCCESSFUL => write!(f, "Successful"),
            BuildState::FAILED => write!(f, "Failed"),
        }
    }
}

#[derive(Serialize, Debug)]
struct ErrorResponse {
    message: String,
}

#[derive(Serialize, Debug, sqlx::FromRow)]
struct Build {
    id: Uuid,
    status: BuildState,
    created_at: DateTime<Utc>,
    finished_at: Option<DateTime<Utc>>,
    branch: Option<String>,
    commit_sha: Option<String>,
}

#[derive(Serialize, Debug)]
struct ProjectBuildListResponse {
    data: Vec<Build>,
}

#[tracing::instrument(skip(auth, pool))]
pub async fn get(
    auth: Auth,
    State(AppState {
        pool,
        domain,
        secure,
        ..
    }): State<AppState>,
    Path((owner, project)): Path<(String, String)>,
) -> Response<Body> {
    let Some(user) = auth.current_user else {
        let json = serde_json::to_string(&ErrorResponse {
            message: "Unauthorized".to_string(),
        })
        .unwrap();
        return Response::builder()
            .status(StatusCode::UNAUTHORIZED)
            .header("Content-Type", "application/json")
            .body(Body::from(json))
            .unwrap();
    };

    // The project query below joins users_owners but never binds the caller, so
    // on its own it only proves that *someone* owns this project. Authorize the
    // caller explicitly before touching any project data.
    match authz::has_project_access(&pool, &owner, &project, user.id).await {
        Ok(true) => {}
        Ok(false) => {
            let json = serde_json::to_string(&ErrorResponse {
                message: "Project not found or you don't have access".to_string(),
            })
            .unwrap();
            return Response::builder()
                .status(StatusCode::NOT_FOUND)
                .header("Content-Type", "application/json")
                .body(Body::from(json))
                .unwrap();
        }
        Err(err) => {
            tracing::error!(?err, "Failed to check project access");
            let json = serde_json::to_string(&ErrorResponse {
                message: "Failed to check project access".to_string(),
            })
            .unwrap();
            return Response::builder()
                .status(StatusCode::INTERNAL_SERVER_ERROR)
                .header("Content-Type", "application/json")
                .body(Body::from(json))
                .unwrap();
        }
    }

    // check if project exist
    let project_record = match sqlx::query!(
        r#"SELECT projects.id, projects.name AS project, project_owners.name AS owner
           FROM projects
           JOIN project_owners ON projects.owner_id = project_owners.id
           JOIN users_owners ON project_owners.id = users_owners.owner_id
           AND projects.name = $1
           AND project_owners.name = $2
        "#,
        project,
        owner,
    )
    .fetch_optional(&pool)
    .await
    {
        Ok(Some(record)) => record,
        Ok(None) => {
            let json = serde_json::to_string(&ErrorResponse {
                message: "Project does not exist".to_string(),
            })
            .unwrap();

            return Response::builder()
                .status(StatusCode::BAD_REQUEST)
                .body(Body::from(json))
                .unwrap();
        }
        Err(err) => {
            tracing::error!(?err, "Can't get projects: Failed to query database");

            let json = serde_json::to_string(&ErrorResponse {
                message: format!("Failed to query database: {}", err.to_string()),
            })
            .unwrap();

            return Response::builder()
                .status(StatusCode::INTERNAL_SERVER_ERROR)
                .body(Body::from(json))
                .unwrap();
        }
    };

    let build_records = match sqlx::query_as::<_, Build>(
        r#"SELECT id, status, created_at, finished_at, branch, commit_sha
        FROM builds WHERE project_id = $1
        ORDER BY created_at DESC"#,
    )
    .bind(project_record.id)
    .fetch_all(&pool)
    .await
    {
        Ok(records) => records,
        Err(err) => {
            let json = serde_json::to_string(&ErrorResponse {
                message: format!("Failed to query database: {}", err.to_string()),
            })
            .unwrap();

            return Response::builder()
                .status(StatusCode::INTERNAL_SERVER_ERROR)
                .body(Body::from(json))
                .unwrap();
        }
    };

    let json = serde_json::to_string(&ProjectBuildListResponse {
        data: build_records,
    })
    .unwrap();

    Response::builder()
        .status(StatusCode::OK)
        .body(Body::from(json))
        .unwrap()
}
