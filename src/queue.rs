use std::{
    collections::{HashSet, VecDeque},
    hash::Hash,
    sync::{
        atomic::{AtomicUsize, Ordering},
        Arc,
    },
    time::{SystemTime, Duration},
};

use anyhow::Result;
use sqlx::PgPool;
use thiserror::Error;
use tokio::sync::mpsc::{self, Receiver, Sender};
use tokio::sync::Mutex;
use tokio::time::{timeout, sleep};
use ulid::Ulid;
use uuid::Uuid;

use crate::{docker::{build_docker, DockerContainer}, configuration::Settings};

type ConcurrentMutex<T> = Arc<Mutex<T>>;

#[derive(Error, Debug)]
#[error("{message:?}")]
pub struct BuildError {
    message: String,
    inner_error: Option<Box<dyn std::error::Error + Send + Sync>>,
}
#[derive(Debug)]
pub struct BuildQueueItem {
    pub container_name: String,
    pub container_src: String,
    pub owner: String,
    pub repo: String,
}

#[derive(Debug)]
pub struct BuildItem {
    pub build_id: Uuid,
    pub container_name: String,
    pub container_src: String,
    pub owner: String,
    pub repo: String,
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
    pub build_count: Arc<AtomicUsize>,
    pub waiting_queue: ConcurrentMutex<VecDeque<BuildItem>>,
    pub waiting_set: ConcurrentMutex<HashSet<String>>,
    pub receive_channel: Receiver<BuildQueueItem>,
    pub pg_pool: PgPool,
    pub config: Settings,
}

impl BuildQueue {
    pub fn new(build_count: usize, pg_pool: PgPool, config: Settings) -> (Self, Sender<BuildQueueItem>) {
        let (tx, rx) = mpsc::channel(32);

        (
            Self {
                build_count: Arc::new(AtomicUsize::new(build_count)),
                waiting_queue: Arc::new(Mutex::new(VecDeque::new())),
                waiting_set: Arc::new(Mutex::new(HashSet::new())),
                receive_channel: rx,
                pg_pool,
                config,
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

    if let Err(err) = sqlx::query!(
        "UPDATE builds set status = 'building' where id = $1",
        build_id
    )
    .execute(&pool)
    .await
    {
        return Err(BuildError {
            message: "Failed to update build status: Failed to query database".to_string(),
            inner_error: Some(Box::new(err)),
        });
    }

    // TODO: Differentiate types of errors returned by build_docker (ex: ImageBuildError, NetworkCreateError, ContainerAttachError)
    let DockerContainer {
        ip, port, ..
    } = match build_docker(&owner, &repo, &container_name, &container_src, pool.clone(), config).await {
        Ok(result) => {
            if let Err(err) = sqlx::query!(
                "UPDATE builds SET status = 'successful', log = $1 WHERE id = $2",
                result.build_log,
                build_id
            )
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
            if let Err(err) = sqlx::query!(
                "UPDATE builds SET status = 'failed', log = $1 WHERE id = $2",
                err.to_string(),
                build_id
            )
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
                   VALUES ($1, $2, $3, $4, $5)"#
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
    build_count: Arc<AtomicUsize>,
    pool: PgPool,
    config: Settings,
) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
    let mut last_metrics_log = SystemTime::now();
    
    loop {
        let mut waiting_queue = waiting_queue.lock().await;
        let mut waiting_set = waiting_set.lock().await;

        let current_build_count = build_count.load(Ordering::SeqCst);
        let queue_len = waiting_queue.len();
        
        // Log metrics every 30 seconds
        if last_metrics_log.elapsed().unwrap_or(Duration::ZERO) > Duration::from_secs(30) {
            tracing::info!(
                "BUILD_QUEUE_METRICS: available_slots={}, queue_length={}, waiting_set_size={}", 
                current_build_count, queue_len, waiting_set.len()
            );
            last_metrics_log = SystemTime::now();
        }

        if current_build_count > 0 && queue_len > 0 {
            let build_item = match waiting_queue.pop_front() {
                Some(build_item) => build_item,
                None => {
                    drop(waiting_queue);
                    drop(waiting_set);
                    continue;
                },
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
            drop(waiting_queue);
            drop(waiting_set);

            {
                let build_count = Arc::clone(&build_count);
                let pool = pool.clone();
                let config = config.clone();
                let build_id = build_item.build_id;
                let container_name = build_item.container_name.clone();

                build_count.fetch_sub(1, Ordering::SeqCst);
                tokio::spawn(async move {
                    let build_start = SystemTime::now();
                    
                    // Add timeout wrapper around trigger_build
                    let build_timeout = Duration::from_secs(config.build.timeout as u64 / 1000); // Convert from ms
                    let build_result = timeout(build_timeout, trigger_build(build_item, pool.clone(), &config)).await;
                    
                    match build_result {
                        Ok(Ok(subdomain)) => {
                            let build_duration = build_start.elapsed().unwrap_or(Duration::ZERO);
                            tracing::info!(
                                "BUILD_SUCCESS: build_id={}, container={}, subdomain={}, duration={}ms", 
                                build_id, container_name, subdomain, build_duration.as_millis()
                            );
                        },
                        Ok(Err(BuildError { message, inner_error })) => {
                            let build_duration = build_start.elapsed().unwrap_or(Duration::ZERO);
                            tracing::error!(
                                "BUILD_ERROR: build_id={}, container={}, duration={}ms, error={}, inner_error={:?}", 
                                build_id, container_name, build_duration.as_millis(), message, inner_error
                            );
                        },
                        Err(_timeout_error) => {
                            tracing::error!(
                                "BUILD_TIMEOUT: build_id={}, container={}, timeout_seconds={}", 
                                build_id, container_name, build_timeout.as_secs()
                            );
                            
                            // Mark build as failed due to timeout
                            let timeout_msg = format!("Build timeout after {} seconds", build_timeout.as_secs());
                            if let Err(err) = sqlx::query!(
                                "UPDATE builds SET status = 'failed', log = $1 WHERE id = $2",
                                timeout_msg,
                                build_id
                            )
                            .execute(&pool)
                            .await
                            {
                                tracing::error!("Failed to update timeout build status: {:?}", err);
                            }
                        }
                    }

                    let final_count = build_count.fetch_add(1, Ordering::SeqCst) + 1;
                    tracing::debug!("BUILD_SLOT_RELEASED: build_id={}, available_slots={}", build_id, final_count);
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
    mut receive_channel: Receiver<BuildQueueItem>,
) {
    while let Some(message) = receive_channel.recv().await {
        let BuildQueueItem {
            container_name,
            container_src,
            owner,
            repo,
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
                    continue;
                }
            },
            Err(err) => {
                tracing::error!(%err, "Can't query project: Failed to query database");
                continue;
            }
        };

        if waiting_set.contains(&container_name) {
            continue;
        }

        let build_id = Uuid::from(Ulid::new());
        match sqlx::query!(
            r#"INSERT INTO builds (id, project_id)
               VALUES ($1, $2)
            "#,
            build_id,
            project.id,
        )
        .fetch_optional(&pool)
        .await
        {
            Ok(build_details) => build_details,
            Err(err) => {
                tracing::error!(%err, "Can't create build: Failed to query database");
                continue;
            }
        };

        let build_item = BuildItem {
            build_id,
            container_name: container_name.clone(),
            container_src,
            owner: owner.clone(),
            repo: repo.clone(),
            created_at: SystemTime::now(),
        };
        
        tracing::info!(
            "BUILD_ENQUEUED: build_id={}, container={}, owner={}, repo={}, queue_position={}", 
            build_id, container_name, owner, repo, waiting_queue.len()
        );

        waiting_set.insert(build_item.container_name.clone());
        waiting_queue.push_back(build_item);
    }
}

pub async fn build_queue_handler(build_queue: BuildQueue) {
    {
        let waiting_queue = Arc::clone(&build_queue.waiting_queue);
        let waiting_set = Arc::clone(&build_queue.waiting_set);
        let pool = build_queue.pg_pool.clone();
        let config = build_queue.config.clone();
        let build_count = Arc::clone(&build_queue.build_count);

        tokio::spawn(async move {
            let _ = process_task_poll(waiting_queue, waiting_set, build_count, pool, config).await;
        });
    }
    {
        let waiting_queue = Arc::clone(&build_queue.waiting_queue);
        let waiting_set = Arc::clone(&build_queue.waiting_set);
        let pool = build_queue.pg_pool.clone();

        tokio::spawn(async move {
            process_task_enqueue(
                waiting_queue,
                waiting_set,
                pool,
                build_queue.receive_channel,
            )
            .await;
        });
    }
}
