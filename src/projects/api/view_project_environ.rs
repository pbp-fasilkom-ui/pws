use axum::extract::{Path, State};
use axum::response::Response;
use hyper::{Body, StatusCode};
use serde::Serialize;
use serde_json::Value;
use uuid::Uuid;

use crate::{auth::Auth, authz, startup::AppState};

#[derive(Serialize, Debug)]
struct EnvironResponse {
    id: Uuid,
    env: Value,
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
    let project = match sqlx::query!(
        r#"SELECT projects.id AS id, projects.name AS project, projects.environs AS env
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

    let json = serde_json::to_string(&EnvironResponse {
        id: project.id,
        env: project.env,
    })
    .unwrap();

    Response::builder()
        .status(StatusCode::OK)
        .body(Body::from(json))
        .unwrap()
}
