use std::{
    collections::{HashSet, VecDeque},
    hash::Hash,
    panic::AssertUnwindSafe,
    sync::Arc,
    time::{Duration, SystemTime},
};

use anyhow::Result;
use futures_util::FutureExt;
use sqlx::PgPool;
use thiserror::Error;
use tokio::sync::mpsc::{self, Receiver, Sender};
use tokio::sync::{oneshot, Mutex, Semaphore};
use tokio::time::{sleep, timeout};
use ulid::Ulid;
use uuid::Uuid;

use crate::{
    build_logs::{BuildLogRegistry, BuildLogState},
    configuration::Settings,
    docker::{build_docker, DockerContainer},
};

type ConcurrentMutex<T> = Arc<Mutex<T>>;

async fn refresh_queue_positions(queue: &VecDeque<BuildItem>) {
    for (index, build) in queue.iter().enumerate() {
        build.log_state.queue_position(Some(index + 1)).await;
    }
}

#[derive(Error, Debug)]
#[error("{message:?}")]
pub struct BuildError {
    message: String,
    inner_error: Option<Box<dyn std::error::Error + Send + Sync>>,
}
#[derive(Debug)]
pub struct BuildQueueItem {
    pub container_name: String,
    pub container_src: Option<String>,
    pub owner: String,
    pub repo: String,
    pub branch: String,
    pub commit_sha: String,
    pub response: Option<oneshot::Sender<Result<Uuid, String>>>,
}

#[derive(Debug)]
pub struct BuildItem {
    pub build_id: Uuid,
    pub container_name: String,
    pub container_src: String,
    pub owner: String,
    pub repo: String,
    pub branch: String,
    pub commit_sha: String,
    pub log_state: Arc<BuildLogState>,
    pub created_at: SystemTime,
}

unsafe impl Send for BuildItem {}
unsafe impl Sync for BuildItem {}

impl Hash for BuildItem {
    fn hash<H: std::hash::Hasher>(&self, state: &mut H) {
        self.container_name.hash(state)
    }
}

impl PartialEq for BuildItem {
    fn eq(&self, other: &Self) -> bool {
        self.container_name == other.container_name
    }
}

impl Eq for BuildItem {}

pub struct BuildQueue {
    pub build_slots: Arc<Semaphore>,
    pub waiting_queue: ConcurrentMutex<VecDeque<BuildItem>>,
    pub waiting_set: ConcurrentMutex<HashSet<String>>,
    pub receive_channel: Receiver<BuildQueueItem>,
    pub pg_pool: PgPool,
    pub config: Settings,
    pub build_logs: BuildLogRegistry,
}

impl BuildQueue {
    pub fn new(
        build_count: usize,
        pg_pool: PgPool,
        config: Settings,
        build_logs: BuildLogRegistry,
    ) -> (Self, Sender<BuildQueueItem>) {
        let (tx, rx) = mpsc::channel(32);

        (
            Self {
                build_slots: Arc::new(Semaphore::new(build_count)),
                waiting_queue: Arc::new(Mutex::new(VecDeque::new())),
                waiting_set: Arc::new(Mutex::new(HashSet::new())),
                receive_channel: rx,
                pg_pool,
                config,
                build_logs,
            },
            tx,
        )
    }
}

pub async fn trigger_build(
    BuildItem {
        build_id,
        owner,
        repo,
        container_src,
        container_name,
        branch: _,
        commit_sha: _,
        log_state,
        created_at: _,
    }: BuildItem,
    pool: PgPool,
    config: &Settings,
) -> Result<String, BuildError> {
    // TODO: need to emmit error somewhere
    let project = match sqlx::query!(
        r#"SELECT projects.id
           FROM projects
           JOIN project_owners ON projects.owner_id = project_owners.id
           WHERE project_owners.name = $1
           AND projects.name = $2
        "#,
        owner,
        repo
    )
    .fetch_optional(&pool)
    .await
    {
        Ok(project) => match project {
            Some(project) => Ok(project),
            None => Err(BuildError {
                message: format!("Project not found with owner {owner} and repo {repo}"),
                inner_error: None,
            }),
        },
        Err(err) => Err(BuildError {
            message: "Can't get project: Failed to query database".to_string(),
            inner_error: Some(Box::new(err)),
        }),
    }?;

    let build_id = match sqlx::query!(
        r#"SELECT builds.id
           FROM builds
           WHERE builds.id = $1
        "#,
        build_id,
    )
    .fetch_optional(&pool)
    .await
    {
        Ok(Some(build)) => Ok(build.id),
        Ok(None) => Err(BuildError {
            message: format!("Failed to find build with id: {build_id}"),
            inner_error: None,
        }),
        Err(err) => Err(BuildError {
            message: "Can't create build: Failed to query database".to_string(),
            inner_error: Some(Box::new(err)),
        }),
    }?;

    if let Err(err) =
        sqlx::query("UPDATE builds SET status = 'building', updated_at = now() WHERE id = $1")
            .bind(build_id)
            .execute(&pool)
            .await
    {
        return Err(BuildError {
            message: "Failed to update build status: Failed to query database".to_string(),
            inner_error: Some(Box::new(err)),
        });
    }
    log_state.status("building").await;

    // TODO: Differentiate types of errors returned by build_docker (ex: ImageBuildError, NetworkCreateError, ContainerAttachError)
    let DockerContainer { ip, port, .. } = match build_docker(
        &owner,
        &repo,
        &container_name,
        &container_src,
        pool.clone(),
        config,
        log_state.clone(),
    )
    .await
    {
        Ok(result) => {
            let (final_log, _, _, _) = log_state.snapshot().await;
            if let Err(err) = sqlx::query(
                "UPDATE builds SET status = 'successful', log = $1, updated_at = now(), finished_at = now() WHERE id = $2"
            )
            .bind(final_log)
            .bind(build_id)
            .execute(&pool)
            .await
            {
                return Err(BuildError {
                    message: "Failed to update build status: Failed to query database".to_string(),
                    inner_error: Some(Box::new(err)),
                });
            }

            Ok(result)
        }
        Err(err) => {
            let error_message = format!("Build failed: {err}\n");
            log_state.append(&error_message).await;
            let (final_log, _, _, _) = log_state.snapshot().await;
            if let Err(err) = sqlx::query(
                "UPDATE builds SET status = 'failed', log = $1, updated_at = now(), finished_at = now() WHERE id = $2"
            )
            .bind(final_log)
            .bind(build_id)
            .execute(&pool)
            .await
            {
                return Err(BuildError {
                    message: format!(
                        "Failed to update build status: Failed to query database: {repo}"
                    ),
                    inner_error: Some(Box::new(err)),
                });
            }

            return Err(BuildError {
                message: format!("A build error occurred while building repository: {repo}"),
                inner_error: None,
            });
        }
    }?;

    // TODO: check why why need this
    let subdomain = match sqlx::query!(
        r#"SELECT domains.name
           FROM domains
           WHERE domains.project_id = $1
        "#,
        project.id
    )
    .fetch_optional(&pool)
    .await
    {
        Ok(Some(subdomain)) => Ok(subdomain.name),
        Ok(None) => {
            let id = Uuid::from(Ulid::new());
            let subdomain = sqlx::query(
                r#"INSERT INTO domains (id, project_id, name, port, docker_ip)
                   VALUES ($1, $2, $3, $4, $5)"#,
            )
            .bind(id)
            .bind(project.id)
            .bind(container_name.clone())
            .bind(port)
            .bind(ip.clone())
            .execute(&pool)
            .await;

            match subdomain {
                Ok(_) => Ok(container_name),
                Err(err) => Err(BuildError {
                    inner_error: Some(Box::new(err)),
                    message: "Can't insert domain: Failed to query database".to_string(),
                }),
            }
        }
        Err(err) => Err(BuildError {
            message: "Can't get subdomain: Failed to query database".to_string(),
            inner_error: Some(Box::new(err)),
        }),
    }?;

    Ok(subdomain)
}

pub async fn process_task_poll(
    waiting_queue: ConcurrentMutex<VecDeque<BuildItem>>,
    waiting_set: ConcurrentMutex<HashSet<String>>,
    build_slots: Arc<Semaphore>,
    pool: PgPool,
    config: Settings,
    build_logs: BuildLogRegistry,
) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
    let mut last_metrics_log = SystemTime::now();

    loop {
        let mut waiting_queue = waiting_queue.lock().await;
        let mut waiting_set = waiting_set.lock().await;

        let available_slots = build_slots.available_permits();
        let queue_len = waiting_queue.len();

        // Log metrics every 30 seconds
        if last_metrics_log.elapsed().unwrap_or(Duration::ZERO) > Duration::from_secs(30) {
            tracing::info!(
                "BUILD_QUEUE_METRICS: available_slots={}, queue_length={}, waiting_set_size={}",
                available_slots,
                queue_len,
                waiting_set.len()
            );
            last_metrics_log = SystemTime::now();
        }

        if available_slots > 0 && queue_len > 0 {
            let permit = match build_slots.clone().try_acquire_owned() {
                Ok(permit) => permit,
                Err(_) => {
                    drop(waiting_queue);
                    drop(waiting_set);
                    continue;
                }
            };
            let build_item = match waiting_queue.pop_front() {
                Some(build_item) => build_item,
                None => {
                    drop(waiting_queue);
                    drop(waiting_set);
                    continue;
                }
            };

            tracing::info!(
                "BUILD_STARTING: build_id={}, container={}, owner={}, repo={}, queue_wait_time={}ms",
                build_item.build_id,
                build_item.container_name,
                build_item.owner,
                build_item.repo,
                build_item.created_at.elapsed().unwrap_or(Duration::ZERO).as_millis()
            );

            waiting_set.remove(&build_item.container_name);
            build_item.log_state.queue_position(None).await;
            refresh_queue_positions(&waiting_queue).await;
            drop(waiting_queue);
            drop(waiting_set);

            {
                let build_slots = Arc::clone(&build_slots);
                let pool = pool.clone();
                let config = config.clone();
                let build_id = build_item.build_id;
                let container_name = build_item.container_name.clone();
                let log_state = build_item.log_state.clone();
                let build_logs = build_logs.clone();

                // The owned permit is released automatically if the task finishes,
                // times out, is aborted, or panics, so queue slots cannot leak.
                tokio::spawn(async move {
                    let build_start = SystemTime::now();

                    // Add timeout wrapper around trigger_build
                    let build_timeout = Duration::from_secs(config.build.timeout as u64 / 1000); // Convert from ms
                    let build_result = AssertUnwindSafe(timeout(
                        build_timeout,
                        trigger_build(build_item, pool.clone(), &config),
                    ))
                    .catch_unwind()
                    .await;

                    match build_result {
                        Ok(Ok(Ok(subdomain))) => {
                            log_state.status("successful").await;
                            let build_duration = build_start.elapsed().unwrap_or(Duration::ZERO);
                            tracing::info!(
                                "BUILD_SUCCESS: build_id={}, container={}, subdomain={}, duration={}ms",
                                build_id, container_name, subdomain, build_duration.as_millis()
                            );
                        }
                        Ok(Ok(Err(BuildError {
                            message,
                            inner_error,
                        }))) => {
                            log_state
                                .append(&format!("Build failed: {message}\n"))
                                .await;
                            let (final_log, _, _, _) = log_state.snapshot().await;
                            if let Err(err) = sqlx::query(
                                "UPDATE builds SET status = 'failed', log = $1, updated_at = now(), finished_at = now() WHERE id = $2"
                            )
                            .bind(final_log)
                            .bind(build_id)
                            .execute(&pool)
                            .await
                            {
                                tracing::error!(?err, %build_id, "Failed to persist failed build log");
                            }
                            log_state.status("failed").await;
                            let build_duration = build_start.elapsed().unwrap_or(Duration::ZERO);
                            tracing::error!(
                                "BUILD_ERROR: build_id={}, container={}, duration={}ms, error={}, inner_error={:?}",
                                build_id, container_name, build_duration.as_millis(), message, inner_error
                            );
                        }
                        Ok(Err(_timeout_error)) => {
                            tracing::error!(
                                "BUILD_TIMEOUT: build_id={}, container={}, timeout_seconds={}",
                                build_id,
                                container_name,
                                build_timeout.as_secs()
                            );

                            // Mark build as failed due to timeout
                            let timeout_msg =
                                format!("Build timeout after {} seconds", build_timeout.as_secs());
                            log_state.append(&format!("{timeout_msg}\n")).await;
                            let (final_log, _, _, _) = log_state.snapshot().await;
                            if let Err(err) = sqlx::query(
                                "UPDATE builds SET status = 'failed', log = $1, updated_at = now(), finished_at = now() WHERE id = $2"
                            )
                            .bind(final_log)
                            .bind(build_id)
                            .execute(&pool)
                            .await
                            {
                                tracing::error!("Failed to update timeout build status: {:?}", err);
                            }
                            log_state.status("failed").await;
                        }
                        Err(panic) => {
                            let panic_message = panic
                                .downcast_ref::<&str>()
                                .map(|message| (*message).to_string())
                                .or_else(|| panic.downcast_ref::<String>().cloned())
                                .unwrap_or_else(|| "unknown panic".to_string());
                            let message =
                                format!("Build aborted by an internal error: {panic_message}\n");
                            log_state.append(&message).await;
                            let (final_log, _, _, _) = log_state.snapshot().await;
                            if let Err(err) = sqlx::query(
                                "UPDATE builds SET status = 'failed', log = $1, updated_at = now(), finished_at = now() WHERE id = $2"
                            )
                            .bind(final_log)
                            .bind(build_id)
                            .execute(&pool)
                            .await
                            {
                                tracing::error!(?err, %build_id, "Failed to persist panicked build");
                            }
                            log_state.status("failed").await;
                            tracing::error!(%build_id, %container_name, %panic_message, "BUILD_PANIC");
                        }
                    }

                    build_logs.write().await.remove(&build_id);

                    drop(permit);
                    tracing::debug!(
                        "BUILD_SLOT_RELEASED: build_id={}, available_slots={}",
                        build_id,
                        build_slots.available_permits()
                    );
                });
            }
        } else {
            drop(waiting_queue);
            drop(waiting_set);
        }
        sleep(Duration::from_millis(5)).await;
    }
    Ok(())
}

pub async fn process_task_enqueue(
    waiting_queue: ConcurrentMutex<VecDeque<BuildItem>>,
    waiting_set: ConcurrentMutex<HashSet<String>>,
    pool: PgPool,
    base: String,
    mut receive_channel: Receiver<BuildQueueItem>,
    build_logs: BuildLogRegistry,
) {
    while let Some(message) = receive_channel.recv().await {
        let BuildQueueItem {
            container_name,
            container_src,
            owner,
            repo,
            branch,
            commit_sha,
            response,
        } = message;
        let mut waiting_queue = waiting_queue.lock().await;
        let mut waiting_set = waiting_set.lock().await;

        let project = match sqlx::query!(
            r#"SELECT projects.id
               FROM projects
               JOIN project_owners ON projects.owner_id = project_owners.id
               WHERE project_owners.name = $1
               AND projects.name = $2
            "#,
            owner,
            repo
        )
        .fetch_optional(&pool)
        .await
        {
            Ok(project) => match project {
                Some(project) => project,
                None => {
                    tracing::error!("Project not found with owner {} and repo {}", owner, repo);
                    if let Some(response) = response {
                        let _ = response.send(Err("Project not found".to_string()));
                    }
                    continue;
                }
            },
            Err(err) => {
                tracing::error!(%err, "Can't query project: Failed to query database");
                if let Some(response) = response {
                    let _ = response.send(Err("Failed to query project".to_string()));
                }
                continue;
            }
        };

        if waiting_set.contains(&container_name) {
            if let Some(response) = response {
                let _ = response.send(Err(
                    "A build for this project is already waiting in the queue".to_string(),
                ));
            }
            continue;
        }

        if response.is_some() {
            let active_build = match sqlx::query_as::<_, (Uuid,)>(
                r#"SELECT id
                   FROM builds
                   WHERE project_id = $1
                     AND status IN ('pending', 'building')
                   ORDER BY created_at DESC
                   LIMIT 1"#,
            )
            .bind(project.id)
            .fetch_optional(&pool)
            .await
            {
                Ok(active_build) => active_build,
                Err(err) => {
                    tracing::error!(%err, "Can't check active project builds");
                    if let Some(response) = response {
                        let _ = response.send(Err("Failed to check active builds".to_string()));
                    }
                    continue;
                }
            };

            if let Some(active_build) = active_build {
                let message = format!("A build is already in progress ({})", active_build.0);
                tracing::info!(project = %project.id, %message, "Skipping duplicate build");
                if let Some(response) = response {
                    let _ = response.send(Err(message));
                }
                continue;
            }
        }

        let (container_src, commit_sha) = match container_src {
            Some(container_src) => (container_src, commit_sha),
            None => {
                let clone_base = base.clone();
                let clone_owner = owner.clone();
                let clone_repo = repo.clone();
                let clone_branch = branch.clone();
                let clone_commit = commit_sha.clone();

                match tokio::task::spawn_blocking(move || {
                    crate::git::prepare_build_source(
                        &clone_base,
                        &clone_owner,
                        &clone_repo,
                        &clone_branch,
                        Some(&clone_commit),
                    )
                })
                .await
                {
                    Ok(Ok(result)) => result,
                    Ok(Err(err)) => {
                        tracing::error!(%err, "Can't prepare redeploy source");
                        if let Some(response) = response {
                            let _ =
                                response.send(Err("Failed to prepare redeploy source".to_string()));
                        }
                        continue;
                    }
                    Err(err) => {
                        tracing::error!(%err, "Redeploy source preparation task failed");
                        if let Some(response) = response {
                            let _ =
                                response.send(Err("Failed to prepare redeploy source".to_string()));
                        }
                        continue;
                    }
                }
            }
        };

        let build_id = Uuid::from(Ulid::new());
        let log_state = BuildLogState::new();
        log_state.status("pending").await;
        build_logs.write().await.insert(build_id, log_state.clone());
        match sqlx::query(
            r#"INSERT INTO builds (id, project_id, branch, commit_sha)
               VALUES ($1, $2, $3, $4)
            "#,
        )
        .bind(build_id)
        .bind(project.id)
        .bind(&branch)
        .bind(&commit_sha)
        .fetch_optional(&pool)
        .await
        {
            Ok(build_details) => build_details,
            Err(err) => {
                build_logs.write().await.remove(&build_id);
                tracing::error!(%err, "Can't create build: Failed to query database");
                if let Some(response) = response {
                    let _ = response.send(Err("Failed to create build".to_string()));
                }
                continue;
            }
        };

        let build_item = BuildItem {
            build_id,
            container_name: container_name.clone(),
            container_src,
            owner: owner.clone(),
            repo: repo.clone(),
            branch: branch.clone(),
            commit_sha: commit_sha.clone(),
            log_state,
            created_at: SystemTime::now(),
        };

        tracing::info!(
            "BUILD_ENQUEUED: build_id={}, container={}, owner={}, repo={}, branch={}, commit={}, queue_position={}",
            build_id, container_name, owner, repo, branch, commit_sha, waiting_queue.len()
        );

        waiting_set.insert(build_item.container_name.clone());
        waiting_queue.push_back(build_item);
        refresh_queue_positions(&waiting_queue).await;

        if let Some(response) = response {
            let _ = response.send(Ok(build_id));
        }
    }
}

pub async fn build_queue_handler(build_queue: BuildQueue) {
    {
        let waiting_queue = Arc::clone(&build_queue.waiting_queue);
        let waiting_set = Arc::clone(&build_queue.waiting_set);
        let pool = build_queue.pg_pool.clone();
        let config = build_queue.config.clone();
        let build_slots = Arc::clone(&build_queue.build_slots);
        let build_logs = build_queue.build_logs.clone();

        tokio::spawn(async move {
            let _ = process_task_poll(
                waiting_queue,
                waiting_set,
                build_slots,
                pool,
                config,
                build_logs,
            )
            .await;
        });
    }
    {
        let waiting_queue = Arc::clone(&build_queue.waiting_queue);
        let waiting_set = Arc::clone(&build_queue.waiting_set);
        let pool = build_queue.pg_pool.clone();
        let base = build_queue.config.git.base.clone();
        let build_logs = build_queue.build_logs.clone();

        tokio::spawn(async move {
            process_task_enqueue(
                waiting_queue,
                waiting_set,
                pool,
                base,
                build_queue.receive_channel,
                build_logs,
            )
            .await;
        });
    }
}
