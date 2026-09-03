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
    Path((owner, repo)): Path<(String, String)>,
    headers: HeaderMap,
    request: Request<B>,
    next: Next<B>,
) -> Result<Response<UnsyncBoxBody<Bytes, axum::Error>>, hyper::Response<Body>> {
    // Validate before the git_auth bypass below, so unauthenticated
    // deployments are covered too. Several handlers downstream build
    // filesystem paths by formatting these segments directly.
    if crate::authz::validate_segment(&owner).is_err()
        || crate::authz::validate_segment(repo.strip_suffix(".git").unwrap_or(&repo)).is_err()
    {
        return Err(Response::builder()
            .status(StatusCode::NOT_FOUND)
            .body(Body::empty())
            .unwrap());
    }

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
        true => { repo.split(".git").next().unwrap_or("") }.to_owned(),
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

            // A malformed header used to panic the request task here, and there
            // is no CatchPanicLayer to absorb it.
            let Ok(decoded) = BASE64.decode(token.as_bytes()) else {
                return Err(auth_err);
            };
            let Ok(decoded) = String::from_utf8(decoded) else {
                return Err(auth_err);
            };
            let mut parts = decoded.split(':');
            let owner_name = parts.next().unwrap_or("");
            let token = parts.next().unwrap_or("");

            // The credentials authenticate `owner_name`, but the repository
            // being acted on belongs to `owner` from the URL. These were never
            // compared, so valid credentials for your own project authorized
            // access to anyone else's project of the same name.
            if owner_name != owner {
                return Err(auth_failed);
            }

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

            // Nothing here may log the presented or stored token. These lines
            // previously ran at info level on every push, and the container's
            // stdout is scraped into Loki, which Grafana can query.
            tracing::debug!(%owner_name, %repo, "Git auth attempt");

            // Check the cheap string comparisons first so that at most one
            // Argon2 verification runs per request.
            let authenticated = tokens
                .iter()
                .filter(|rec| rec.project_name == repo && rec.project_owner == owner_name)
                .any(|rec| verify_token(token, &rec.token));

            if !authenticated {
                return Err(auth_failed);
            }

            Ok(next.run(request).await)
        }
    }
}

/// Verifies a presented git token against the stored Argon2 hash.
///
/// Tokens used to be stored and compared as plaintext, so anything that could
/// read the database or the logs obtained working push credentials, and the
/// `==` comparison also leaked their contents through response timing.
///
/// A stored value that is not a valid PHC string is refused rather than
/// compared literally: that would reintroduce plaintext comparison for any row
/// the migration missed.
fn verify_token(presented: &str, stored: &str) -> bool {
    let Ok(parsed) = PasswordHash::new(stored) else {
        tracing::error!("Stored git token is not a valid hash; refusing to authenticate");
        return false;
    };

    Argon2::default()
        .verify_password(presented.as_bytes(), &parsed)
        .is_ok()
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
    let pushed_branches = receive_pack_updated_branches(&headers, &body);

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

    // A tag-only push or branch deletion should not trigger an application build.
    let Some(deploy_branch) = select_deploy_branch(pushed_branches) else {
        tracing::info!("No updated branch found in receive-pack; skipping deployment");
        return res;
    };

    let container_name = format!("{owner}-{}", repo.trim_end_matches(".git")).replace('.', "-");

    let (container_src, head_commit_id) =
        match prepare_build_source(&base, &owner, &repo, &deploy_branch, None) {
            Ok(result) => result,
            Err(err) => {
                tracing::error!(%err, "Failed to prepare pushed source");
                return Response::builder()
                    .status(StatusCode::INTERNAL_SERVER_ERROR)
                    .body(Body::empty())
                    .unwrap();
            }
        };

    tokio::spawn(async move {
        build_channel
            .send(BuildQueueItem {
                container_name,
                container_src: Some(container_src),
                owner,
                repo,
                branch: deploy_branch,
                commit_sha: head_commit_id,
                response: None,
            })
            .await
    });

    res
}

/// Clone a project repository into the build workspace and detach it at the
/// requested commit. When no commit is supplied, the tip of `branch` is used.
pub fn prepare_build_source(
    base: &str,
    owner: &str,
    repo: &str,
    branch: &str,
    expected_commit: Option<&str>,
) -> Result<(String, String)> {
    let path = if repo.ends_with(".git") {
        format!("{base}/{owner}/{repo}")
    } else {
        format!("{base}/{owner}/{repo}.git")
    };
    let container_src = format!("{path}/clone");

    let bare_repo = git2::Repository::open_bare(&path)?;
    let commit_id = match expected_commit {
        Some(commit) => {
            let commit_id = git2::Oid::from_str(commit)?;
            bare_repo.find_commit(commit_id)?;
            commit_id
        }
        None => bare_repo
            .revparse_single(&format!("refs/heads/{branch}"))?
            .id(),
    };

    if std::path::Path::new(&container_src).exists() {
        tracing::info!("Removing existing working directory: {}", container_src);
        std::fs::remove_dir_all(&container_src)?;
    }

    tracing::info!("Creating fresh clone from bare repo to: {}", container_src);
    let mut repo_builder = git2::build::RepoBuilder::new();
    repo_builder.branch(branch);
    let cloned_repo = repo_builder.clone(&path, std::path::Path::new(&container_src))?;
    cloned_repo.set_head_detached(commit_id)?;
    cloned_repo.checkout_head(Some(git2::build::CheckoutBuilder::default().force()))?;

    tracing::info!(branch, commit = %commit_id, "Prepared build source");
    Ok((container_src, commit_id.to_string()))
}

fn receive_pack_updated_branches(headers: &HeaderMap, body: &Bytes) -> Vec<String> {
    let decoded;
    let body = match headers
        .get("Content-Encoding")
        .and_then(|value| value.to_str().ok())
    {
        Some("gzip") => {
            let mut reader = flate2::read::GzDecoder::new(body.as_ref());
            let mut bytes = Vec::new();
            if reader.read_to_end(&mut bytes).is_err() {
                return Vec::new();
            }
            decoded = bytes;
            decoded.as_slice()
        }
        _ => body.as_ref(),
    };

    let mut branches = Vec::new();
    let mut offset = 0;

    while offset + 4 <= body.len() {
        let Ok(length_text) = std::str::from_utf8(&body[offset..offset + 4]) else {
            break;
        };
        let Ok(length) = usize::from_str_radix(length_text, 16) else {
            break;
        };
        offset += 4;

        if length == 0 {
            continue;
        }
        if length < 4 || offset + length - 4 > body.len() {
            break;
        }

        let payload = &body[offset..offset + length - 4];
        offset += length - 4;
        let command = payload.split(|byte| *byte == 0).next().unwrap_or(payload);
        let Ok(command) = std::str::from_utf8(command) else {
            continue;
        };
        let mut fields = command.split_whitespace();
        let _old_id = fields.next();
        let new_id = fields.next();
        let reference = fields.next();

        if let (Some(new_id), Some(reference)) = (new_id, reference) {
            if !new_id.bytes().all(|byte| byte == b'0') {
                if let Some(branch) = reference.strip_prefix("refs/heads/") {
                    if !branches.iter().any(|existing| existing == branch) {
                        branches.push(branch.to_string());
                    }
                }
            }
        }
    }

    branches
}

fn select_deploy_branch(mut branches: Vec<String>) -> Option<String> {
    for preferred in ["main", "master"] {
        if let Some(index) = branches.iter().position(|branch| branch == preferred) {
            return Some(branches.remove(index));
        }
    }
    branches.into_iter().next()
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
