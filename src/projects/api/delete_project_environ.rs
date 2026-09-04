use axum::body::Body;
use axum::extract::{Path, State};
use axum::http::StatusCode;
use axum::response::Response;
use axum::Json;
use garde::{Unvalidated, Validate};
use serde::{Deserialize, Serialize};

use crate::{auth::Auth, authz, startup::AppState};

#[derive(Deserialize, Validate, Debug)]
pub struct DeleteProjectEnvironRequest {
    #[garde(length(min = 1))]
    pub key: String,
}

#[derive(Serialize, Debug)]
struct ErrorResponse {
    message: String,
}

#[tracing::instrument(skip(auth, pool))]
pub async fn post(
    auth: Auth,
    State(AppState {
        pool,
        domain,
        secure,
        ..
    }): State<AppState>,
    Path((owner, project)): Path<(String, String)>,
    Json(req): Json<Unvalidated<DeleteProjectEnvironRequest>>,
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

    let DeleteProjectEnvironRequest { key } = match req.validate(&()) {
        Ok(valid) => valid.into_inner(),
        Err(err) => {
            let json = serde_json::to_string(&ErrorResponse {
                message: err.to_string(),
            })
            .unwrap();

            return Response::builder()
                .status(StatusCode::BAD_REQUEST)
                .body(Body::from(json))
                .unwrap();
        }
    };

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

    match sqlx::query!(
        r#"UPDATE projects
            SET environs = environs - $1
            WHERE id = $2
        "#,
        key,
        project.id
    )
    .execute(&pool)
    .await
    {
        Ok(data) => data,
        Err(err) => {
            tracing::error!(
                ?err,
                "Can't delete project environs: Failed to insert into database"
            );

            let json = serde_json::to_string(&ErrorResponse {
                message: "Failed to insert into database".to_string(),
            })
            .unwrap();

            return Response::builder()
                .status(StatusCode::INTERNAL_SERVER_ERROR)
                .body(Body::from(json))
                .unwrap();
        }
    };

    Response::builder()
        .status(StatusCode::NO_CONTENT)
        .body(Body::empty())
        .unwrap()
}
