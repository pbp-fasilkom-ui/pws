use axum::{
    extract::{Path, Query, State},
    response::Response,
};
use git2::{ObjectType, Repository};
use hyper::{Body, StatusCode};
use serde::{Deserialize, Serialize};
use std::path::Path as StdPath;

use crate::{auth::Auth, startup::AppState};

const MAX_FILE_SIZE: usize = 512 * 1024;

#[derive(Debug, Deserialize)]
pub struct FileQuery {
    #[serde(rename = "ref")]
    r#ref: Option<String>,
    path: String,
}

#[derive(Serialize)]
struct FileResponse {
    #[serde(rename = "ref")]
    r#ref: String,
    path: String,
    size: usize,
    content: String,
}

#[tracing::instrument(skip(auth, pool, base))]
pub async fn get(
    auth: Auth,
    State(AppState { pool, base, .. }): State<AppState>,
    Path((owner, project)): Path<(String, String)>,
    Query(query): Query<FileQuery>,
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

    if query.path.is_empty() || StdPath::new(&query.path).is_absolute() {
        return json_error(StatusCode::BAD_REQUEST, "Invalid file path");
    }

    let repo_path = format!("{base}/{owner}/{}.git", project.trim_end_matches(".git"));
    let repo = match Repository::open_bare(repo_path) {
        Ok(repo) => repo,
        Err(error) => return json_error(StatusCode::INTERNAL_SERVER_ERROR, &error.to_string()),
    };
    let ref_name = query.r#ref.unwrap_or_else(|| "HEAD".to_string());
    let commit = match repo
        .revparse_single(&ref_name)
        .and_then(|object| object.peel_to_commit())
    {
        Ok(commit) => commit,
        Err(_) => return json_error(StatusCode::BAD_REQUEST, "Invalid reference"),
    };
    let tree = match commit.tree() {
        Ok(tree) => tree,
        Err(_) => return json_error(StatusCode::INTERNAL_SERVER_ERROR, "Failed to read tree"),
    };
    let entry = match tree.get_path(StdPath::new(&query.path)) {
        Ok(entry) if entry.kind() == Some(ObjectType::Blob) => entry,
        _ => return json_error(StatusCode::NOT_FOUND, "File not found"),
    };
    let blob = match repo.find_blob(entry.id()) {
        Ok(blob) => blob,
        Err(_) => return json_error(StatusCode::INTERNAL_SERVER_ERROR, "Failed to read file"),
    };

    if blob.size() > MAX_FILE_SIZE {
        return json_error(
            StatusCode::PAYLOAD_TOO_LARGE,
            "File is larger than the 512 KiB preview limit",
        );
    }
    if blob.content().contains(&0) {
        return json_error(StatusCode::UNSUPPORTED_MEDIA_TYPE, "Binary files cannot be previewed");
    }
    let content = match std::str::from_utf8(blob.content()) {
        Ok(content) => content.to_owned(),
        Err(_) => return json_error(StatusCode::UNSUPPORTED_MEDIA_TYPE, "File is not valid UTF-8"),
    };

    let body = serde_json::to_string(&FileResponse {
        r#ref: ref_name,
        path: query.path,
        size: blob.size(),
        content,
    })
    .unwrap();
    Response::builder()
        .status(StatusCode::OK)
        .header("Content-Type", "application/json")
        .body(Body::from(body))
        .unwrap()
}

fn json_error(status: StatusCode, message: &str) -> Response<Body> {
    Response::builder()
        .status(status)
        .header("Content-Type", "application/json")
        .body(Body::from(serde_json::json!({ "message": message }).to_string()))
        .unwrap()
}
