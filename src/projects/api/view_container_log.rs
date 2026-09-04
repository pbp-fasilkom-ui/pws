use axum::body::Body;
use axum::extract::{Path, State};
use axum::http::StatusCode;
use axum::response::Response;
use bollard::container::{LogOutput, LogsOptions};
use bollard::Docker;
use futures::StreamExt;
use serde::Serialize;
use uuid::Uuid;

use crate::{auth::Auth, authz, startup::AppState};

#[derive(Serialize, Debug)]
struct LogResponse {
    id: Uuid,
    logs: String,
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
        r#"SELECT projects.id, domains.name AS container_name
           FROM projects
           JOIN project_owners ON projects.owner_id = project_owners.id
           JOIN users_owners ON project_owners.id = users_owners.owner_id
           JOIN domains ON domains.project_id = projects.id
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

    let docker = match Docker::connect_with_local_defaults().map_err(|err| {
        tracing::error!("Failed to connect to docker: {}", err);
        err
    }) {
        Ok(docker) => docker,
        Err(err) => {
            tracing::error!(?err, "Failed to connect to docker");

            let json = serde_json::to_string(&ErrorResponse {
                message: format!("Failed to connect to docker: {}", err.to_string()),
            })
            .unwrap();

            return Response::builder()
                .status(StatusCode::INTERNAL_SERVER_ERROR)
                .body(Body::from(json))
                .unwrap();
        }
    };

    let log_stream = &mut docker.logs(
        &project.container_name,
        Some(LogsOptions {
            tail: "100",
            stdout: true,
            stderr: true,
            ..Default::default()
        }),
    );
    let mut logs = String::new();

    while let Some(log_result) = log_stream.next().await {
        match log_result {
            Ok(log_output) => match log_output {
                LogOutput::StdOut { message } | LogOutput::StdErr { message } => {
                    logs.push_str(&String::from_utf8_lossy(&message));
                }
                _ => {}
            },
            Err(e) => eprintln!("Error: {}", e), // Error handling
        }
    }

    let json = serde_json::to_string(&LogResponse {
        id: project.id,
        logs: logs,
    })
    .unwrap();

    Response::builder()
        .status(StatusCode::OK)
        .body(Body::from(json))
        .unwrap()
}
