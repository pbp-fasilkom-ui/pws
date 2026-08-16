use axum::{
    extract::{Path, State},
    response::Response,
};
use git2::{BranchType, Repository};
use hyper::{Body, StatusCode};
use serde::Serialize;

use crate::{auth::Auth, startup::AppState};

#[derive(Serialize)]
struct RefsResponse {
    default_branch: Option<String>,
    deployed_branch: Option<String>,
    branches: Vec<String>,
}

#[tracing::instrument(skip(auth, pool, base))]
pub async fn get(
    auth: Auth,
    State(AppState { pool, base, .. }): State<AppState>,
    Path((owner, project)): Path<(String, String)>,
) -> Response<Body> {
    let Some(user) = auth.current_user else {
        return json_error(StatusCode::UNAUTHORIZED, "Unauthorized");
    };

    let has_access = sqlx::query_scalar::<_, bool>(
        r#"
        SELECT EXISTS (
          SELECT 1
          FROM projects
          JOIN project_owners ON projects.owner_id = project_owners.id
          LEFT JOIN users_owners ON project_owners.id = users_owners.owner_id
          LEFT JOIN project_shares ON projects.id = project_shares.project_id
          WHERE projects.name = $1
            AND project_owners.name = $2
            AND projects.deleted_at IS NULL
            AND (users_owners.user_id = $3 OR project_shares.user_id = $3)
        )
        "#,
    )
    .bind(&project)
    .bind(&owner)
    .bind(user.id)
    .fetch_one(&pool)
    .await
    .unwrap_or(false);

    if !has_access {
        return json_error(StatusCode::NOT_FOUND, "Project not found or access denied");
    }

    let repo_path = format!("{base}/{owner}/{}.git", project.trim_end_matches(".git"));
    let repo = match Repository::open_bare(repo_path) {
        Ok(repo) => repo,
        Err(error) => {
            return json_error(
                StatusCode::INTERNAL_SERVER_ERROR,
                &format!("Failed to open repository: {error}"),
            )
        }
    };

    let default_branch = repo
        .head()
        .ok()
        .and_then(|reference| reference.shorthand().map(str::to_owned));
    let deployed_branch = sqlx::query_scalar::<_, Option<String>>(
        r#"
        SELECT builds.branch
        FROM builds
        JOIN projects ON builds.project_id = projects.id
        JOIN project_owners ON projects.owner_id = project_owners.id
        WHERE projects.name = $1
          AND project_owners.name = $2
          AND builds.status = 'successful'
          AND builds.branch IS NOT NULL
        ORDER BY builds.created_at DESC
        LIMIT 1
        "#,
    )
    .bind(&project)
    .bind(&owner)
    .fetch_optional(&pool)
    .await
    .ok()
    .flatten()
    .flatten();
    let mut branches = repo
        .branches(Some(BranchType::Local))
        .map(|iter| {
            iter.filter_map(|entry| {
                let (branch, _) = entry.ok()?;
                branch.name().ok().flatten().map(str::to_owned)
            })
            .collect::<Vec<_>>()
        })
        .unwrap_or_default();
    branches.sort_unstable();

    let body = serde_json::to_string(&RefsResponse {
        default_branch,
        deployed_branch,
        branches,
    })
    .unwrap();
    Response::builder()
        .status(StatusCode::OK)
        .header("Content-Type", "application/json")
        .body(Body::from(body))
        .unwrap()
}

fn json_error(status: StatusCode, message: &str) -> Response<Body> {
    let body = serde_json::json!({ "message": message }).to_string();
    Response::builder()
        .status(status)
        .header("Content-Type", "application/json")
        .body(Body::from(body))
        .unwrap()
}
