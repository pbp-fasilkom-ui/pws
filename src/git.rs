use std::{
    ffi::OsStr,
    fs::File,
    io::Read,
    path::Path as StdPath,
    process::{Output, Stdio},
};

use argon2::{
    password_hash::{PasswordHash, PasswordVerifier},
    Argon2,
};
use axum::{
    extract::{DefaultBodyLimit, Path, Query, State},
    middleware::{self, Next},
    response::Response,
    routing::{get, post},
    Router,
};
use axum_extra::routing::RouterExt;
use git2::Repository;
use http_body::combinators::UnsyncBoxBody;
use hyper::{
    body::Bytes, http::response::Builder as ResponseBuilder, Body, HeaderMap, Request, StatusCode,
};

use anyhow::Result;
use serde::Deserialize;
use tokio::{io::AsyncWriteExt, process::Command};
use tower_http::limit::RequestBodyLimitLayer;

use crate::{configuration::Settings, queue::BuildQueueItem, startup::AppState};

use data_encoding::BASE64;

async fn basic_auth<B>(
    State(AppState { pool, git_auth, .. }): State<AppState>,
    Path((_owner, repo)): Path<(String, String)>,
    headers: HeaderMap,
    request: Request<B>,
    next: Next<B>,
) -> Result<Response<UnsyncBoxBody<Bytes, axum::Error>>, hyper::Response<Body>> {
    if !git_auth {
        return Ok(next.run(request).await);
    }

    let auth_err = Response::builder()
        .status(StatusCode::UNAUTHORIZED)
        .header("WWW-Authenticate", "Basic realm=\"git\"")
        .body(Body::empty())
        .unwrap();

    let auth_failed = Response::builder()
        .status(StatusCode::UNAUTHORIZED)
        .header("WWW-Authenticate", "Basic realm=\"failed to login\"")
        .body(Body::empty())
        .unwrap();

    let repo = match repo.ends_with(".git") {
        true => {
            repo.split(".git").next().unwrap_or("")
        }.to_owned(),
        false => format!("{repo}"),
    };

    match headers.get("Authorization").and_then(|v| v.to_str().ok()) {
        None => Err(auth_err),
        Some(auth) => {
            let mut parts = auth.split_whitespace();
            let scheme = parts.next().unwrap_or("");
            let token = parts.next().unwrap_or("");

            if scheme != "Basic" {
                return Err(auth_err);
            }

            let decoded = BASE64.decode(token.as_bytes()).unwrap();
            let decoded = String::from_utf8(decoded).unwrap();
            let mut parts = decoded.split(':');
            let owner_name = parts.next().unwrap_or("");
            let token = parts.next().unwrap_or("");

            let tokens = match sqlx::query!(
                r#"SELECT projects.name AS project_name, api_token.token AS token, project_owners.name AS project_owner
                    FROM project_owners
                    JOIN projects ON project_owners.id = projects.owner_id
                    JOIN api_token ON projects.id = api_token.project_id
                    WHERE project_owners.name = $1
                "#,
                owner_name
            )
            .fetch_all(&pool)
            .await
            {
                Ok(tokens) => tokens,
                Err(sqlx::Error::RowNotFound) => return Err(auth_failed),
                Err(_) => return Err(auth_err),
            };

            tracing::debug!("AUTH_DEBUG: Auth attempt - owner: {}, repo: {}, token: {}", owner_name, repo, token);
            tracing::debug!("AUTH_DEBUG: Found {} tokens in database", tokens.len());
            
            let authenticated = tokens.iter().any(|rec| {
                tracing::info!("Checking token - project: {}, owner: {}, stored_token: {}", rec.project_name, rec.project_owner, rec.token);
                
                // Use plain text comparison instead of argon2 hashing
                let token_match = rec.token == token;
                let authorization_match = rec.project_name == repo && rec.project_owner == owner_name;
                
                tracing::info!("Token match: {}, Authorization match: {}", token_match, authorization_match);

                token_match && authorization_match
            });
            
            if !authenticated {
                return Err(auth_failed);
            }

            Ok(next.run(request).await)
        }
    }
}

pub fn router(state: AppState, config: &Settings) -> Router<AppState, Body> {
    Router::new()
        .route_with_tsr("/:owner/:repo/git-upload-pack", post(upload_pack_rpc))
        .route_with_tsr("/:owner/:repo/git-receive-pack", post(receive_pack_rpc))
        .route_with_tsr("/:owner/:repo/info/refs", get(get_info_refs))
        .route_with_tsr(
            "/:owner/:repo/HEAD",
            get(
                |Path((owner, repo)): Path<(String, String)>,
                 State(AppState { base, .. }): State<AppState>| async move {
                    get_file_text(&base, &owner, &repo, "HEAD").await
                },
            ),
        )
        .route_with_tsr(
            "/:owner/:repo/objects/info/alternates",
            get(
                |Path((owner, repo)): Path<(String, String)>,
                 State(AppState { base, .. }): State<AppState>| async move {
                    get_file_text(&base, &owner, &repo, "objects/info/alternates").await
                },
            ),
        )
        .route_with_tsr(
            "/:owner/:repo/objects/info/http-alternates",
            get(
                |Path((owner, repo)): Path<(String, String)>,
                 State(AppState { base, .. }): State<AppState>| async move {
                    get_file_text(&base, &owner, &repo, "objects/info/http-alternates").await
                },
            ),
        )
        .route_with_tsr("/:owner/:repo/objects/info/packs", get(get_info_packs))
        .route_with_tsr(
            "/:owner/:repo/objects/info/:file",
            get(
                |Path((owner, repo, head, file)): Path<(String, String, String, String)>,
                 State(AppState { base, .. }): State<AppState>| async move {
                    get_file_text(&base, &owner, &repo, format!("{}/{}", head, file).as_ref()).await
                },
            ),
        )
        .route_with_tsr("/:owner/:repo/objects/:head/:hash", get(get_loose_object))
        .route_with_tsr(
            "/:owner/:repo/objects/packs/:file",
            get(get_pack_or_idx_file),
        )
        .route_layer(middleware::from_fn_with_state(state, basic_auth))
        // not git server related
        .layer(DefaultBodyLimit::disable())
        .layer(RequestBodyLimitLayer::new(config.body_limit()))
    // .with_state(state)
}

async fn git_command<P, IA, S, IE, K, V>(dir: P, args: IA, envs: IE) -> Result<Output>
where
    P: AsRef<StdPath>,
    IA: IntoIterator<Item = S>,
    S: AsRef<OsStr>,
    IE: IntoIterator<Item = (K, V)>,
    K: AsRef<OsStr>,
    V: AsRef<OsStr>,
{
    let output = Command::new("git")
        .current_dir(dir)
        .args(args)
        .envs(envs)
        .output()
        .await?;

    Ok(output)
}

fn get_git_service(service: &str) -> &str {
    match service.starts_with("git-") {
        true => &service[4..],
        false => "",
    }
}

fn packet_write(s: &str) -> Vec<u8> {
    let length = s.len() + 4;
    let mut length_hex = format!("{:x}", length);

    while length_hex.len() % 4 != 0 {
        length_hex.insert(0, '0');
    }

    let result = format!("{}{}", length_hex, s);

    result.into_bytes()
}

fn packet_flush() -> Vec<u8> {
    "0000".into()
}

trait GitServer {
    fn no_cache(self) -> Self;
    fn cache_forever(self) -> Self;
}

impl GitServer for ResponseBuilder {
    fn no_cache(self) -> Self {
        self.header("Expires", "Fri, 01 Jan 1980 00:00:00 GMT")
            .header("Pragma", "no-cache")
            .header("Cache-Control", "no-cache, max-age=0, must-revalidate")
    }
    fn cache_forever(self) -> Self {
        let now = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_millis();

        let expire = now + 31536000;
        self.header("Date", now.to_string().as_str())
            .header("Expires", expire.to_string().as_str())
            .header("Cache-Control", "public, max-age=31536000")
    }
}

pub async fn get_info_packs(
    Path(repo): Path<String>,
    State(AppState { base, .. }): State<AppState>,
) -> Response<Body> {
    let path = match repo.ends_with(".git") {
        true => format!("{base}/{repo}/objects/info/packs"),
        false => format!("{base}/{repo}.git/objects/info/packs"),
    };

    let mut file = match File::open(path) {
        Ok(file) => file,
        Err(_) => return Response::builder().status(404).body(Body::empty()).unwrap(),
    };

    let mut contents = String::new();
    file.read_to_string(&mut contents).unwrap();
    Response::builder()
        .no_cache()
        .header("Content-Type", "text/plain; charset=utf-8")
        .body(Body::from(contents))
        .unwrap()
}

pub async fn get_loose_object(
    Path((repo, head, hash)): Path<(String, String, String)>,
    State(AppState { base, .. }): State<AppState>,
) -> Response<Body> {
    let path = match repo.ends_with(".git") {
        true => format!("{base}/{repo}/objects/{head}/{hash}"),
        false => format!("{base}/{repo}.git/objects/{head}{hash}"),
    };
    let mut file = match File::open(path) {
        Ok(file) => file,
        Err(_) => return Response::builder().status(404).body(Body::empty()).unwrap(),
    };

    let mut contents = Vec::new();
    file.read_to_end(&mut contents).unwrap();

    Response::builder()
        .cache_forever()
        .header("Content-Type", "application/x-git-loose-object")
        .body(Body::from(contents))
        .unwrap()
}

pub async fn get_pack_or_idx_file(
    Path((repo, file)): Path<(String, String)>,
    State(AppState { base, .. }): State<AppState>,
) -> Response<Body> {
    let path = match repo.ends_with(".git") {
        true => format!("{base}/{repo}/objects/pack/{file}"),
        false => format!("{base}/{repo}.git/objects/pack{file}"),
    };
    let mut file = match File::open(&path) {
        Ok(file) => file,
        Err(_) => return Response::builder().status(404).body(Body::empty()).unwrap(),
    };

    let res = Response::builder().cache_forever();

    let res = match StdPath::new(&path).extension().and_then(|ext| ext.to_str()) {
        Some("pack") => res.header("Content-Type", "application/x-git-packed-objects"),
        Some("idx") => res.header("Content-Type", "application/x-git-packed-objects-toc"),
        _ => return Response::builder().status(404).body(Body::empty()).unwrap(),
    };

    let mut contents = Vec::new();
    file.read_to_end(&mut contents).unwrap();

    res.body(Body::from(contents)).unwrap()
}

pub async fn get_file_text(base: &str, owner: &str, repo: &str, file: &str) -> Response<Body> {
    let path = match repo.ends_with(".git") {
        true => format!("{base}/{owner}/{repo}/{file}"),
        false => format!("{base}/{owner}/{repo}.git/{file}"),
    };

    let mut file = match File::open(path) {
        Ok(file) => file,
        Err(_) => return Response::builder().status(404).body(Body::empty()).unwrap(),
    };

    let mut contents = String::new();
    file.read_to_string(&mut contents).unwrap();
    Response::builder()
        .header("Content-Type", "text/plain")
        .body(Body::from(contents))
        .unwrap()
}

pub async fn receive_pack_rpc(
    Path((owner, repo)): Path<(String, String)>,
    State(AppState {
        base,
        build_channel,
        ..
    }): State<AppState>,
    headers: HeaderMap,
    body: Bytes,
) -> Response<Body> {
    let path = match repo.ends_with(".git") {
        true => format!("{base}/{owner}/{repo}"),
        false => format!("{base}/{owner}/{repo}.git"),
    };
    let head_dir = format!("{path}/refs/heads");

    let res = service_rpc("receive-pack", &path, headers, body).await;
    if res.status() != StatusCode::OK {
        return res;
    }
    if res
        .headers()
        .get("Content-Length")
        .and_then(|k| k.to_str().ok())
        .and_then(|k| k.eq("0").then_some(()))
        .is_some()
    {
        return res;
    }

    let container_src = format!("{path}/clone");
    let container_name = format!("{owner}-{}", repo.trim_end_matches(".git")).replace('.', "-");

    // FIXED: Get HEAD commit directly from bare repo to ensure consistency 
    // This resolves the issue where copy directory was out of sync with tree view
    let bare_repo_path = if repo.ends_with(".git") {
        format!("{base}/{owner}/{repo}")
    } else {
        format!("{base}/{owner}/{repo}.git")
    };
    
    let head_commit_id = match git2::Repository::open_bare(&bare_repo_path) {
        Ok(bare_repo) => {
            match bare_repo.revparse_single("HEAD") {
                Ok(obj) => {
                    let commit_id = obj.id();
                    tracing::info!("Got HEAD commit from bare repo: {}", commit_id);
                    commit_id
                },
                Err(e) => {
                    tracing::error!("Failed to resolve HEAD in bare repo: {}", e);
                    return Response::builder()
                        .status(StatusCode::INTERNAL_SERVER_ERROR)
                        .body(Body::empty())
                        .unwrap();
                }
            }
        },
        Err(e) => {
            tracing::error!("Failed to open bare repo: {}", e);
            return Response::builder()
                .status(StatusCode::INTERNAL_SERVER_ERROR)
                .body(Body::empty())
                .unwrap();
        }
    };

    // Always fresh clone to guarantee up-to-date state
    // Delete existing working directory if it exists
    if std::path::Path::new(&container_src).exists() {
        tracing::info!("Removing existing working directory: {}", container_src);
        if let Err(e) = std::fs::remove_dir_all(&container_src) {
            tracing::error!("Failed to remove existing directory: {}", e);
        }
    }
    
    // Fresh clone from bare repo - always up-to-date
    tracing::info!("Creating fresh clone from bare repo to: {}", container_src);
    match git2::Repository::clone(&path, &container_src) {
        Ok(cloned_repo) => {
            tracing::info!("Fresh clone completed, now setting to exact HEAD commit");
            
            // Set to exact same commit as HEAD in bare repo (matching tree view)
            if let Err(e) = cloned_repo.set_head_detached(head_commit_id) {
                tracing::error!("Failed to set cloned repo HEAD: {}", e);
            } else {
                // Force checkout to make working directory match
                if let Err(e) = cloned_repo.checkout_head(Some(
                    git2::build::CheckoutBuilder::default().force()
                )) {
                    tracing::error!("Failed to checkout cloned repo HEAD: {}", e);
                } else {
                    tracing::info!("Successfully set working directory to commit: {}", head_commit_id);
                }
            }
        },
        Err(e) => {
            tracing::error!("Fresh clone failed: {}", e);
            return Response::builder()
                .status(StatusCode::INTERNAL_SERVER_ERROR)
                .body(Body::empty())
                .unwrap();
        }
    }


    tokio::spawn(async move {
        build_channel
            .send(BuildQueueItem {
                container_name,
                container_src,
                owner,
                repo,
            })
            .await
    });

    res
}

pub async fn upload_pack_rpc(
    Path((owner, repo)): Path<(String, String)>,
    State(AppState { base, .. }): State<AppState>,
    headers: HeaderMap,
    body: Bytes,
) -> Response<Body> {
    let path = match repo.ends_with(".git") {
        true => format!("{base}/{owner}/{repo}"),
        false => format!("{base}/{owner}/{repo}.git"),
    };

    service_rpc("upload-pack", &path, headers, body).await
}

pub async fn service_rpc(rpc: &str, path: &str, headers: HeaderMap, body: Bytes) -> Response<Body> {
    let mut response = Response::builder()
        .header("Content-Type", format!("application/x-git-{rpc}-result"))
        .body(Body::empty())
        .unwrap();

    let body = match headers
        .get("Content-Encoding")
        .and_then(|enc| enc.to_str().ok())
    {
        Some("gzip") => {
            let mut reader = flate2::read::GzDecoder::new(body.as_ref());
            let mut new_bytes = Vec::new();
            match reader.read_to_end(&mut new_bytes) {
                Ok(_) => Bytes::from(new_bytes),
                Err(_) => {
                    *response.status_mut() = StatusCode::INTERNAL_SERVER_ERROR;
                    return response;
                }
            }
        }
        _ => body,
    };

    if body == b"0000".as_slice() {
        response
            .headers_mut()
            .insert("Vary", "Accept-Encoding".parse().unwrap());
        response
            .headers_mut()
            .insert("Content-Length", "0".parse().unwrap());
        return response;
    }

    let env = match headers.get("Git-Protocol").and_then(|v| v.to_str().ok()) {
        Some("version=2") => ("GIT_PROTOCOL".to_string(), "version=2".to_string()),
        _ => ("".to_string(), "".to_string()),
    };

    let envs = std::env::vars().chain([env]).collect::<Vec<_>>();

    let mut cmd = Command::new("git");
    cmd.args([rpc, "--stateless-rpc", path])
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .envs(envs);

    let mut child = cmd.spawn().expect("failed to spawn command");
    let mut stdin = child.stdin.take().expect("failed to get stdin");

    if let Err(e) = stdin.write_all(&body).await {
        tracing::error!("Failed to write to stdin: {}", e);
        *response.status_mut() = StatusCode::INTERNAL_SERVER_ERROR;
        return response;
    }
    drop(stdin);

    let output = child
        .wait_with_output()
        .await
        .expect("Failed to read stdout/stderr");

    if !output.status.success() {
        tracing::error!("Command failed: {:?}", output.status);
        tracing::error!("Stderr: {}", String::from_utf8_lossy(&output.stderr));
        *response.status_mut() = StatusCode::INTERNAL_SERVER_ERROR;
    } else {
        tracing::info!("Command succeeded!");
        tracing::info!("Stdout: {}", String::from_utf8_lossy(&output.stdout));
        tracing::info!("Stderr: {}", String::from_utf8_lossy(&output.stderr));
        *response.body_mut() = Body::from(output.stdout);
    }

    response
}

#[derive(Deserialize, Debug)]
pub struct GitQuery {
    service: String,
}

pub async fn get_info_refs(
    Path((owner, repo)): Path<(String, String)>,
    State(AppState { base, .. }): State<AppState>,
    Query(GitQuery { service }): Query<GitQuery>,
    headers: HeaderMap,
) -> Response<Body> {
    let service = get_git_service(&service);

    let path = match repo.ends_with(".git") {
        true => format!("{base}/{owner}/{repo}"),
        false => format!("{base}/{owner}/{repo}.git"),
    };
    if service != "receive-pack" && service != "upload-pack" {
        git_command(
            &path,
            &["update-server-info"],
            std::iter::empty::<(String, String)>(),
        )
        .await
        .unwrap();

        let mut file = match File::open(path) {
            Ok(file) => file,
            Err(_) => return Response::builder().status(404).body(Body::empty()).unwrap(),
        };

        let mut contents = String::new();
        file.read_to_string(&mut contents).unwrap();

        return Response::builder()
            .no_cache()
            .header("Content-Type", "text/plain; charset=utf-8")
            .body(Body::from(contents))
            .unwrap();
    }

    let env = match headers.get("Git-Protocol").and_then(|v| v.to_str().ok()) {
        Some("version=2") => ("GIT_PROTOCOL".to_string(), "version=2".to_string()),
        _ => ("".to_string(), "".to_string()),
    };

    let envs = std::env::vars().chain([env]).collect::<Vec<_>>();

    let out = match git_command(
        &path,
        &[service, "--stateless-rpc", "--advertise-refs", "."],
        envs,
    )
    .await
    {
        Ok(out) => out,
        Err(err) => {
            tracing::error!(path, service, ?err, "Failed to run git command: {}", err);
            return Response::builder()
                .status(StatusCode::INTERNAL_SERVER_ERROR)
                .body(Body::empty())
                .unwrap();
        }
    };

    let body = packet_write(&format!("# service=git-{}\n", service));
    let body = [body, packet_flush(), out.stdout].concat();

    Response::builder()
        .no_cache()
        .header(
            "Content-Type",
            format!("application/x-git-{service}-advertisement"),
        )
        .header("Vary", "Accept-Encoding")
        .header("Accept-Encoding", "Chunked")
        .body(Body::from(body))
        .unwrap()
}
