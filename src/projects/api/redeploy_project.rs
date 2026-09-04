use axum::extract::{Path, State};
use axum::response::Response;
use hyper::{Body, StatusCode};
use serde::Serialize;
use tokio::sync::oneshot;
use uuid::Uuid;

use crate::{auth::Auth, authz, queue::BuildQueueItem, startup::AppState};

#[derive(Serialize)]
struct RedeployResponse {
    build_id: Uuid,
    branch: String,
    commit_sha: String,
}

#[derive(Serialize)]
struct ErrorResponse {
    message: String,
}

fn json_response<T: Serialize>(status: StatusCode, body: T) -> Response<Body> {
    Response::builder()
        .status(status)
        .header("Content-Type", "application/json")
        .body(Body::from(serde_json::to_string(&body).unwrap()))
        .unwrap()
}

fn error_response(status: StatusCode, message: impl Into<String>) -> Response<Body> {
    json_response(
        status,
        ErrorResponse {
            message: message.into(),
        },
    )
}

#[tracing::instrument(skip(auth, pool, build_channel))]
pub async fn post(
    auth: Auth,
    State(AppState {
        pool,
        build_channel,
        ..
    }): State<AppState>,
    Path((owner, project)): Path<(String, String)>,
) -> Response<Body> {
    let Some(user) = auth.current_user else {
        return error_response(StatusCode::UNAUTHORIZED, "Unauthorized");
    };

    let project_id = match sqlx::query_as::<_, (Uuid,)>(
        r#"SELECT projects.id
           FROM projects
           JOIN project_owners ON projects.owner_id = project_owners.id
           LEFT JOIN users_owners ON project_owners.id = users_owners.owner_id
           LEFT JOIN project_shares ON projects.id = project_shares.project_id
           WHERE projects.name = $1
             AND project_owners.name = $2
             AND projects.deleted_at IS NULL
             AND (users_owners.user_id = $3 OR project_shares.user_id = $3)
        "#,
    )
    .bind(&project)
    .bind(&owner)
    .bind(user.id)
    .fetch_optional(&pool)
    .await
    {
        Ok(Some((project_id,))) => project_id,
        Ok(None) => {
            return error_response(StatusCode::NOT_FOUND, "Project not found or inaccessible")
        }
        Err(err) => {
            tracing::error!(?err, "Failed to check redeploy project access");
            return error_response(StatusCode::INTERNAL_SERVER_ERROR, "Failed to query project");
        }
    };

    match sqlx::query_as::<_, (Uuid,)>(
        r#"SELECT id
           FROM builds
           WHERE project_id = $1
             AND status IN ('pending', 'building')
           ORDER BY created_at DESC
           LIMIT 1"#,
    )
    .bind(project_id)
    .fetch_optional(&pool)
    .await
    {
        Ok(Some((build_id,))) => {
            return error_response(
                StatusCode::CONFLICT,
                format!("A build is already in progress ({build_id})"),
            )
        }
        Ok(None) => {}
        Err(err) => {
            tracing::error!(?err, %project_id, "Failed to check active builds");
            return error_response(
                StatusCode::INTERNAL_SERVER_ERROR,
                "Failed to check active builds",
            );
        }
    }

    let (branch, commit_sha) = match sqlx::query_as::<_, (String, String)>(
        r#"SELECT branch, commit_sha
           FROM builds
           WHERE project_id = $1
             AND branch IS NOT NULL
             AND commit_sha IS NOT NULL
           ORDER BY created_at DESC
           LIMIT 1"#,
    )
    .bind(project_id)
    .fetch_optional(&pool)
    .await
    {
        Ok(Some(revision)) => revision,
        Ok(None) => {
            return error_response(
                StatusCode::CONFLICT,
                "This project has no revision that can be redeployed",
            )
        }
        Err(err) => {
            tracing::error!(?err, %project_id, "Failed to get latest project revision");
            return error_response(
                StatusCode::INTERNAL_SERVER_ERROR,
                "Failed to get latest project revision",
            );
        }
    };

    let container_name = match authz::container_name(&owner, &project) {
        Ok(name) => name,
        Err(err) => {
            tracing::error!(%owner, %project, %err, "Refusing to redeploy a reserved name");
            return error_response(StatusCode::BAD_REQUEST, "Invalid project");
        }
    };
    let (response_sender, response_receiver) = oneshot::channel();

    if build_channel
        .send(BuildQueueItem {
            container_name,
            container_src: None,
            owner,
            repo: project,
            branch: branch.clone(),
            commit_sha: commit_sha.clone(),
            response: Some(response_sender),
        })
        .await
        .is_err()
    {
        return error_response(
            StatusCode::SERVICE_UNAVAILABLE,
            "Build queue is unavailable",
        );
    }

    match response_receiver.await {
        Ok(Ok(build_id)) => json_response(
            StatusCode::ACCEPTED,
            RedeployResponse {
                build_id,
                branch,
                commit_sha,
            },
        ),
        Ok(Err(message)) => error_response(StatusCode::CONFLICT, message),
        Err(err) => {
            tracing::error!(?err, "Build queue closed before redeploy was accepted");
            error_response(
                StatusCode::SERVICE_UNAVAILABLE,
                "Build queue is unavailable",
            )
        }
    }
}
