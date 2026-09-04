use crate::auth::Auth;
use crate::authz;
use crate::startup::AppState;
use axum::extract::{Path, State};
use axum::response::Response;
use bollard::container::{StartContainerOptions, StopContainerOptions};
use bollard::Docker;
use hyper::{Body, StatusCode};
use serde::Serialize;

#[derive(Serialize)]
struct DeleteVolumeSuccessResponse {
    message: String,
}

#[derive(Serialize)]
struct DeleteVolumeErrorResponse {
    message: String,
    details: Vec<String>,
}

#[tracing::instrument(skip(auth, pool))]
pub async fn post(
    auth: Auth,
    State(AppState { pool, .. }): State<AppState>,
    Path((owner, project)): Path<(String, String)>,
) -> Response<Body> {
    fn deny(status: StatusCode, message: &str) -> Response<Body> {
        let json = serde_json::to_string(&DeleteVolumeErrorResponse {
            message: message.to_string(),
            details: vec![],
        })
        .unwrap();

        Response::builder()
            .status(status)
            .header("Content-Type", "application/json")
            .body(Body::from(json))
            .unwrap()
    }

    // As in delete_project: deny on a missing user rather than falling through,
    // and authorize against users_owners instead of comparing the username to
    // the URL segment.
    let Some(user) = auth.current_user else {
        return deny(StatusCode::UNAUTHORIZED, "Unauthorized");
    };

    match authz::is_project_owner(&pool, &owner, &project, user.id).await {
        Ok(true) => {}
        Ok(false) => {
            return deny(
                StatusCode::NOT_FOUND,
                "Project not found or you don't have access",
            )
        }
        Err(err) => {
            tracing::error!(?err, "Can't delete volume: Failed to check ownership");
            return deny(
                StatusCode::INTERNAL_SERVER_ERROR,
                "Failed to check project ownership",
            );
        }
    }

    // Derived from validated segments; refuses names that would resolve to a
    // platform container, so registering the username `db` cannot target the
    // database container.
    let container_name = match authz::container_name(&owner, &project) {
        Ok(name) => name,
        Err(err) => {
            tracing::warn!(%owner, %project, %err, "Rejected volume deletion target");
            return deny(StatusCode::BAD_REQUEST, "Invalid project");
        }
    };
    let db_name = format!("{}-db", container_name);
    let volume_name = format!("{}-volume", container_name);

    let docker = match Docker::connect_with_local_defaults() {
        Ok(docker) => docker,
        Err(err) => {
            tracing::error!(?err, "Can't delete volume: Failed to connect to docker");
            // TODO: better message
            return Response::builder()
                .status(StatusCode::INTERNAL_SERVER_ERROR)
                .body(Body::from(""))
                .unwrap();
        }
    };

    let turned_on = match docker.inspect_container(&db_name, None).await {
        Ok(_) => {
            match docker
                .stop_container(&db_name, None::<StopContainerOptions>)
                .await
            {
                Ok(_) => true,
                Err(err) => {
                    tracing::error!(?err, "Can't delete volume: Failed to stop db");
                    false
                }
            }
        }
        Err(err) => {
            tracing::debug!(?err, "Can't delete volume: db does not exist");
            false
        }
    };

    let status = match docker.inspect_volume(&volume_name).await {
        Ok(_) => match docker.remove_volume(&volume_name, None).await {
            Ok(_) => "successfully deleted",
            Err(err) => {
                tracing::error!(?err, "Can't delete volume: Failed to delete volume");
                "failed to delete: volume error"
            }
        },
        Err(err) => {
            tracing::debug!(?err, "Can't delete volume: volume does not exist");
            "failed to delete: volume does not exist"
        }
    };

    if turned_on {
        match docker
            .start_container(&db_name, None::<StartContainerOptions<&str>>)
            .await
        {
            Ok(_) => {}
            Err(err) => {
                tracing::error!(?err, "Can't delete volume: Failed to start db");
            }
        }
    }

    let json = serde_json::to_string(&DeleteVolumeSuccessResponse {
        message: status.to_string(),
    })
    .unwrap();

    Response::builder()
        .status(StatusCode::OK)
        .body(Body::from(json))
        .unwrap()
}
