use std::collections::HashMap;
use std::fs::File;

use axum::extract::{Path, State};
use axum::response::Response;
use bollard::container::{RemoveContainerOptions, StopContainerOptions};
use bollard::Docker;
use hyper::{Body, StatusCode};
use serde::Serialize;

use crate::auth::Auth;
use crate::authz;
use crate::startup::AppState;

#[derive(Serialize)]
struct DeleteProjectSuccessResponse {
    message: String,
}

#[derive(Serialize)]
struct DeleteProjectErrorResponse {
    message: String,
    details: Vec<String>,
}

#[tracing::instrument(skip(pool, base, auth))]
pub async fn post(
    auth: Auth,
    Path((owner, project)): Path<(String, String)>,
    State(AppState { pool, base, .. }): State<AppState>,
) -> Response<Body> {
    fn to_response(status: HashMap<&'static str, &'static str>) -> Response<Body> {
        let success = status.iter().all(|(_, v)| *v == "successfully deleted");
        let json = match success {
            true => serde_json::to_string(&DeleteProjectSuccessResponse {
                message: "Successfully deleted project".to_string(),
            }),
            false => serde_json::to_string(&DeleteProjectErrorResponse {
                message: "Failed to delete project".to_string(),
                details: status
                    .into_iter()
                    .map(|(k, v)| format!("{}: {}", k.to_string(), v.to_string()))
                    .collect::<Vec<_>>(),
            }),
        }
        .unwrap();

        Response::builder()
            .status(StatusCode::OK)
            .body(Body::from(json))
            .unwrap()
    }

    fn deny(status: StatusCode, message: &str) -> Response<Body> {
        let json = serde_json::to_string(&DeleteProjectErrorResponse {
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

    // A missing user used to fall through to the deletion below. Deny instead:
    // the auth middleware makes this unreachable today, but the handler should
    // not depend on the router to stay safe.
    let Some(user) = auth.current_user else {
        return deny(StatusCode::UNAUTHORIZED, "Unauthorized");
    };

    // Authorize against users_owners rather than comparing the username to the
    // URL segment, which is how every other handler does it.
    match authz::is_project_owner(&pool, &owner, &project, user.id).await {
        Ok(true) => {}
        Ok(false) => {
            return deny(
                StatusCode::NOT_FOUND,
                "Project not found or you don't have access",
            )
        }
        Err(err) => {
            tracing::error!(?err, "Can't delete project: Failed to check ownership");
            return deny(
                StatusCode::INTERNAL_SERVER_ERROR,
                "Failed to check project ownership",
            );
        }
    }

    // Built from validated segments and checked to stay under `base`. The old
    // format! took the URL parameters verbatim, so a percent-encoded `../` in
    // `project` reached remove_dir_all below and deleted another user's repo.
    let path = match authz::repo_path(&base, &owner, &project) {
        Ok(path) => path,
        Err(err) => {
            tracing::warn!(%owner, %project, %err, "Rejected project deletion path");
            return deny(StatusCode::BAD_REQUEST, "Invalid project");
        }
    };

    //TODO: better error log
    let mut status: HashMap<&'static str, &'static str> = HashMap::new();

    // check if owner exist
    match sqlx::query!(
        r#"SELECT id FROM project_owners WHERE name = $1 AND deleted_at IS NULL"#,
        owner,
    )
    .fetch_optional(&pool)
    .await
    {
        Ok(Some(data)) => {
            // check if project exist
            match sqlx::query!(
                r#"SELECT id FROM projects WHERE name = $1 AND owner_id = $2"#,
                project,
                data.id,
            )
            .fetch_optional(&pool)
            .await
            {
                Ok(Some(_)) => {
                    match sqlx::query!(
                        "DELETE FROM projects WHERE name = $1 AND owner_id = $2",
                        project,
                        data.id
                    )
                    .execute(&pool)
                    .await
                    {
                        Ok(_) => {
                            status.insert("project", "successfully deleted");
                        }
                        Err(err) => {
                            tracing::error!(?err, "Can't delete project: Failed to delete project");
                            status.insert("project", "failed to delete: database error");
                        }
                    }
                }
                Err(err) => {
                    tracing::error!(?err, "Can't delete project: Failed to query database");
                    status.insert("project", "failed to delete: database error");
                }
                _ => {
                    status.insert("project", "failed to delete: project does not exist");
                }
            };
        }
        Ok(None) => {
            tracing::debug!("Can't delete project: Owner does not exist");
        }
        Err(err) => {
            tracing::error!(?err, "Can't get project_owners: Failed to query database");
        }
    }

    // check if repo exists
    match File::open(&path) {
        Err(err) => {
            tracing::debug!(?err, "Can't delete project: Repo does not exist");
            status.insert("repo", "failed to delete: repo does not exist");
        }
        Ok(_) => match std::fs::remove_dir_all(&path) {
            Ok(_) => {
                status.insert("repo", "successfully deleted");
            }
            Err(err) => {
                tracing::error!(?err, "Can't delete project: Failed to delete repo");
                status.insert("repo", "failed to delete: repo error");
            }
        },
    };

    let container_name = match authz::container_name(&owner, &project) {
        Ok(name) => name,
        Err(err) => {
            tracing::warn!(%owner, %project, %err, "Rejected project deletion target");
            return deny(StatusCode::BAD_REQUEST, "Invalid project");
        }
    };

    let docker = match Docker::connect_with_local_defaults() {
        Err(err) => {
            tracing::error!(?err, "Can't delete project: Failed to connect to docker");
            status.insert("container", "failed to delete: docker error");
            return to_response(status);
        }
        Ok(docker) => docker,
    };

    // remove container
    match docker.inspect_container(&container_name, None).await {
        Ok(_) => {
            match docker
                .stop_container(&container_name, None::<StopContainerOptions>)
                .await
            {
                Ok(_) => {
                    match docker
                        .remove_container(&container_name, None::<RemoveContainerOptions>)
                        .await
                    {
                        Ok(_) => {
                            status.insert("container", "successfully deleted");
                        }
                        Err(err) => {
                            tracing::error!(
                                ?err,
                                "Can't delete project: Failed to delete container"
                            );
                            status.insert("container", "failed to delete: container error");
                        }
                    }
                }
                Err(err) => {
                    tracing::error!(?err, "Can't delete project: Failed to stop container");
                    status.insert("container", "failed to delete: container error");
                }
            };
        }
        Err(err) => {
            tracing::debug!(?err, "Can't delete project: Container does not exist");
            status.insert("container", "failed to delete: container does not exist");
        }
    };

    // remove image
    match docker.inspect_image(&container_name).await {
        Ok(_) => match docker.remove_image(&container_name, None, None).await {
            Ok(_) => {
                status.insert("image", "successfully deleted");
            }
            Err(err) => {
                tracing::error!(?err, "Can't delete project: Failed to delete image");
                status.insert("image", "failed to delete: image error");
            }
        },
        Err(err) => {
            tracing::debug!(?err, "Can't delete project: Image does not exist");
            status.insert("image", "failed to delete: image does not exist");
        }
    };

    to_response(status)
}
