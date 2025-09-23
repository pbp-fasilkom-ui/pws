use axum::extract::{State, Path};
use axum::response::Response;
use hyper::{Body, StatusCode};
use serde::Serialize;

use crate::{auth::Auth, startup::AppState};
use sqlx::Row;
use uuid::Uuid;

#[derive(Serialize, Debug)]
struct GitCredentialsResponse {
    git_username: String,
    git_url: String,
    project_name: String,
    owner_name: String,
}

#[derive(Serialize, Debug)]
struct ErrorResponse {
    message: String,
}

#[tracing::instrument(skip(auth, pool))]
pub async fn get(
    auth: Auth,
    State(AppState { pool, domain, secure, .. }): State<AppState>,
    Path((owner, project)): Path<(String, String)>,
) -> Response<Body> {
    let Some(user) = auth.current_user else {
        let json = serde_json::to_string(&ErrorResponse {
            message: "Unauthorized".to_string(),
        }).unwrap();
        return Response::builder()
            .status(StatusCode::UNAUTHORIZED)
            .header(axum::http::header::CONTENT_TYPE, "application/json")
            .body(Body::from(json))
            .unwrap();
    };

    // check if project exist and user has access (owner or shared)
    let row = sqlx::query(
        r#"SELECT projects.id, projects.name AS project, project_owners.name AS owner
           FROM projects
           JOIN project_owners ON projects.owner_id = project_owners.id
           LEFT JOIN users_owners ON project_owners.id = users_owners.owner_id
           LEFT JOIN project_shares ON projects.id = project_shares.project_id
           WHERE projects.name = $1
             AND project_owners.name = $2
             AND (users_owners.user_id = $3 OR project_shares.user_id = $3)
        "#,
    )
    .bind(&project)
    .bind(&owner)
    .bind(user.id)
    .fetch_optional(&pool)
    .await;

    let project_record = match row {
        Ok(Some(r)) => {
            struct Rec { id: Uuid, project: String, owner: String }
            Rec {
                id: r.get::<Uuid, _>("id"),
                project: r.get::<String, _>("project"),
                owner: r.get::<String, _>("owner"),
            }
        }
        Ok(None) => {
            let json = serde_json::to_string(&ErrorResponse {
                message: "Project does not exist or you don't have access".to_string(),
            }).unwrap();

            return Response::builder()
                .status(StatusCode::NOT_FOUND)
                .header(axum::http::header::CONTENT_TYPE, "application/json")
                .body(Body::from(json))
                .unwrap();
        }
        Err(err) => {
            tracing::error!(?err, "Can't get project: Failed to query database");

            let json = serde_json::to_string(&ErrorResponse {
                message: "Internal server error".to_string(),
            }).unwrap();

            return Response::builder()
                .status(StatusCode::INTERNAL_SERVER_ERROR)
                .header(axum::http::header::CONTENT_TYPE, "application/json")
                .body(Body::from(json))
                .unwrap();
        }
    };

    let protocol = match secure {
        true => "https",
        false => "http",
    };

    let git_url = format!("{protocol}://{domain}/{owner}/{project}");

    let json = serde_json::to_string(&GitCredentialsResponse {
        git_username: user.username,
        git_url,
        project_name: project_record.project,
        owner_name: project_record.owner,
    }).unwrap();

    Response::builder()
        .status(StatusCode::OK)
        .header(axum::http::header::CONTENT_TYPE, "application/json")
        .body(Body::from(json))
        .unwrap()
}
