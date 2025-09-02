use axum::{
    extract::{Path, Query, State},
    response::Response,
};
use hyper::{Body, StatusCode};
use serde::Serialize;
use git2::{ObjectType, Repository};
use std::path::Path as StdPath;

use crate::startup::AppState;

#[derive(Serialize, Debug)]
#[serde(tag = "kind", rename_all = "lowercase")]
pub enum TreeEntry {
    Dir { name: String },
    File { name: String, size: u64 },
    Symlink { name: String },
    Submodule { name: String },
    Other { name: String },
}

#[derive(Serialize, Debug)]
pub struct TreeResponse {
    #[serde(rename = "ref")]
    r#ref: String,
    path: String,
    is_empty_repo: bool,
    entries: Vec<TreeEntry>,
}

#[derive(Debug, serde::Deserialize)]
pub struct TreeQuery {
    /// Branch, tag, or commit hash (defaults to "HEAD")
    #[serde(rename = "ref")]
    r#ref: Option<String>,
    /// Directory path within the repo (defaults to root)
    path: Option<String>,
}

#[tracing::instrument(skip(pool, base))]
pub async fn get(
    Path((owner, project)): Path<(String, String)>,
    State(AppState { pool, base, .. }): State<AppState>,
    Query(TreeQuery { r#ref, path }): Query<TreeQuery>,
) -> Response<Body> {
    // ---- Project existence (runtime SQLx; no macros -> no DATABASE_URL at build) ----
    
    
    let exists = sqlx::query_scalar::<_, bool>(
        r#"
        SELECT EXISTS (
          SELECT 1
          FROM projects
          JOIN project_owners ON projects.owner_id = project_owners.id
          WHERE project_owners.name = $1
            AND projects.name = $2
            AND projects.deleted_at IS NULL
        )
        "#
    )
    .bind(&owner)
    .bind(&project)
    .fetch_one(&pool)
    .await;

    let exists = match exists {
        Ok(v) => v,
        Err(err) => {
            let body = serde_json::to_string(&serde_json::json!({
                "message": format!("Database error: {}", err)
            })).unwrap();
            return Response::builder()
                .status(StatusCode::INTERNAL_SERVER_ERROR)
                .header("Content-Type", "application/json")
                .body(Body::from(body))
                .unwrap();
        }
    };

    if !exists {
        let body = serde_json::to_string(&serde_json::json!({
            "message": "Project not found"
        })).unwrap();
        return Response::builder()
            .status(StatusCode::NOT_FOUND)
            .header("Content-Type", "application/json")
            .body(Body::from(body))
            .unwrap();
    }



    // ---- Open bare repository ----
    let repo_path = if project.ends_with(".git") {
        format!("{base}/{owner}/{project}")
    } else {
        format!("{base}/{owner}/{project}.git")
    };

    let repo = match Repository::open_bare(&repo_path) {
        Ok(r) => r,
        Err(err) => {
            let body = serde_json::to_string(&serde_json::json!({
                "message": format!("Failed to open repository: {}", err)
            }))
            .unwrap();
            return Response::builder()
                .status(StatusCode::INTERNAL_SERVER_ERROR)
                .header("Content-Type", "application/json")
                .body(Body::from(body))
                .unwrap();
        }
    };

    // ---- Resolve ref (default HEAD); handle unborn HEAD (empty repo) ----
    let ref_input = r#ref.unwrap_or_else(|| "HEAD".to_string());
    let (is_empty_repo, tree_opt) = match repo.revparse_single(&ref_input) {
        Ok(obj) => {
            if let Ok(commit) = obj.peel_to_commit() {
                (false, Some(commit.tree().ok()))
            } else if let Ok(tree) = obj.peel_to_tree() {
                (false, Some(Some(tree)))
            } else {
                (false, None)
            }
        }
        Err(_) => {
            // Unborn HEAD => empty repo
            if repo.head().ok().and_then(|h| h.target()).is_none() {
                (true, None)
            } else {
                let body = serde_json::to_string(&serde_json::json!({
                    "message": "Invalid reference"
                }))
                .unwrap();
                return Response::builder()
                    .status(StatusCode::BAD_REQUEST)
                    .header("Content-Type", "application/json")
                    .body(Body::from(body))
                    .unwrap();
            }
        }
    };

    if is_empty_repo {
        let json = serde_json::to_string(&TreeResponse {
            r#ref: ref_input,
            path: path.clone().unwrap_or_default(),
            is_empty_repo: true,
            entries: vec![],
        })
        .unwrap();
        return Response::builder()
            .status(StatusCode::OK)
            .header("Content-Type", "application/json")
            .body(Body::from(json))
            .unwrap();
    }

    let mut tree = match tree_opt.flatten() {
        Some(t) => t,
        None => {
            let body = serde_json::to_string(&serde_json::json!({
                "message": "Reference is not a tree/commit"
            }))
            .unwrap();
            return Response::builder()
                .status(StatusCode::BAD_REQUEST)
                .header("Content-Type", "application/json")
                .body(Body::from(body))
                .unwrap();
        }
    };

    // ---- Traverse into subdirectory if path provided ----
    let path_str = path.unwrap_or_default();
    if !path_str.is_empty() {
        match tree.get_path(StdPath::new(&path_str)) {
            Ok(entry) => {
                let obj = match entry.to_object(&repo) {
                    Ok(o) => o,
                    Err(_) => {
                        let body = serde_json::to_string(&serde_json::json!({
                            "message": "Path not found"
                        }))
                        .unwrap();
                        return Response::builder()
                            .status(StatusCode::NOT_FOUND)
                            .header("Content-Type", "application/json")
                            .body(Body::from(body))
                            .unwrap();
                    }
                };
                match obj.as_tree() {
                    Some(t) => tree = t.clone(),
                    None => {
                        let body = serde_json::to_string(&serde_json::json!({
                            "message": "Path is not a directory"
                        }))
                        .unwrap();
                        return Response::builder()
                            .status(StatusCode::BAD_REQUEST)
                            .header("Content-Type", "application/json")
                            .body(Body::from(body))
                            .unwrap();
                    }
                }
            }
            Err(_) => {
                let body = serde_json::to_string(&serde_json::json!({
                    "message": "Path not found"
                }))
                .unwrap();
                return Response::builder()
                    .status(StatusCode::NOT_FOUND)
                    .header("Content-Type", "application/json")
                    .body(Body::from(body))
                    .unwrap();
            }
        }
    }


    // ---- Collect and sort entries: dirs, files, symlinks, submodules, others ----
    let mut entries: Vec<TreeEntry> = Vec::new();

    for entry in tree.iter() {
        let name = entry
            .name()
            .map(|s| s.to_string())
            .unwrap_or_else(|| String::from_utf8_lossy(entry.name_bytes()).to_string());

        match entry.kind() {
            Some(ObjectType::Tree) => entries.push(TreeEntry::Dir { name }),
            Some(ObjectType::Commit) => entries.push(TreeEntry::Submodule { name }),
            Some(ObjectType::Blob) => {
                // 0o120000 is a symlink in git trees
                if entry.filemode() == 0o120000 {
                    entries.push(TreeEntry::Symlink { name });
                } else {
                    let size = repo.find_blob(entry.id()).map(|b| b.size() as u64).unwrap_or(0);
                    entries.push(TreeEntry::File { name, size });
                }
            }
            _ => entries.push(TreeEntry::Other { name }),
        }
    }

    // Sort by (rank, lowercase_name). Using owned key avoids lifetimes.
    entries.sort_by_key(|e| {
        use TreeEntry::*;
        let rank: u8 = match e {
            Dir { .. } => 0,
            File { .. } => 1,
            Symlink { .. } => 2,
            Submodule { .. } => 3,
            Other { .. } => 4,
        };
        let name = match e {
            Dir { name }
            | File { name, .. }
            | Symlink { name }
            | Submodule { name }
            | Other { name } => name.to_lowercase(),
        };
        (rank, name)
    });

    // ---- Respond ----
    let json = serde_json::to_string(&TreeResponse {
        r#ref: ref_input,
        path: path_str,
        is_empty_repo: false,
        entries,
    })
    .unwrap();

    Response::builder()
        .status(StatusCode::OK)
        .header("Content-Type", "application/json")
        .body(Body::from(json))
        .unwrap()
}

