use std::fmt;

use axum::extract::{Path, State};
use axum::response::Response;
use hyper::{Body, StatusCode};
use serde::{Deserialize, Serialize};

use crate::{auth::Auth, startup::AppState};

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
    // check if project exist
    let project_record = match sqlx::query!(
        r#"SELECT projects.id
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

    let build = match sqlx::query!(
        r#"SELECT id, project_id, status AS "status: BuildState", created_at, updated_at, finished_at, log
        FROM builds WHERE project_id = $1
        ORDER BY created_at DESC"#,
        project_record.id
    )
    .fetch_optional(&pool)
    .await
    {
        Ok(record) => record,
        Err(err) => {
            // This route is unauthenticated, so the sqlx error text -- which
            // names tables, columns and constraints -- must not be returned.
            tracing::error!(?err, "Failed to query build status for badge");
            let json = serde_json::to_string(&ErrorResponse {
                message: "Failed to query database".to_string(),
            })
            .unwrap();

            return Response::builder()
                .status(StatusCode::INTERNAL_SERVER_ERROR)
                .body(Body::from(json))
                .unwrap();
        }
    };

    let mut style = badgen::Style::flat();

    // A project that has never been built is a normal state, not an error;
    // fetch_one turned it into a 500 for every freshly created project.
    let (label, colour) = match build.as_ref().map(|b| &b.status) {
        Some(BuildState::PENDING) => ("pending", badgen::Color::Grey),
        Some(BuildState::FAILED) => ("failed", badgen::Color::Red),
        Some(BuildState::SUCCESSFUL) => ("successful", badgen::Color::Green),
        Some(BuildState::BUILDING) => ("building", badgen::Color::Yellow),
        None => ("no builds", badgen::Color::Grey),
    };
    style.background = colour;

    let badge = badgen::badge(&style, label, Some("PWS Build Status")).unwrap();

    let mut response = Response::builder()
        .status(StatusCode::OK)
        .header("Content-Type", "image/svg+xml")
        .header("Cache-Control", "no-cache");

    // Only meaningful once a build exists.
    if let Some(build) = build.as_ref() {
        response = response.header("Last-Modified", build.updated_at.to_rfc2822());
    }

    response.body(Body::from(badge)).unwrap()
}
