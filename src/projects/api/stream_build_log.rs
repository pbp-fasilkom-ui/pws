use std::{collections::VecDeque, convert::Infallible, sync::Arc, time::Duration};

use axum::http::StatusCode;
use axum::{
    extract::{Path, State},
    response::{
        sse::{Event, KeepAlive, Sse},
        IntoResponse, Response,
    },
};
use futures::stream;
use serde::Serialize;
use tokio::sync::broadcast;
use uuid::Uuid;

use crate::{
    auth::Auth,
    build_logs::{BuildLogEvent, BuildLogState},
    startup::AppState,
};

#[derive(Serialize)]
struct Snapshot<'a> {
    logs: &'a str,
    sequence: u64,
}

#[derive(Serialize)]
struct Chunk<'a> {
    chunk: &'a str,
    sequence: u64,
}

#[derive(Serialize)]
struct Status<'a> {
    status: &'a str,
    sequence: u64,
}

#[derive(Serialize)]
struct QueuePosition {
    position: Option<usize>,
    sequence: u64,
}

struct StreamState {
    initial: VecDeque<Event>,
    receiver: Option<broadcast::Receiver<BuildLogEvent>>,
    log_state: Option<Arc<BuildLogState>>,
    snapshot_sequence: u64,
}

fn json_event<T: Serialize>(event: &str, value: T) -> Event {
    Event::default()
        .event(event)
        .json_data(value)
        .unwrap_or_else(|_| Event::default().event("error").data("serialization failed"))
}

pub async fn get(
    auth: Auth,
    State(AppState {
        pool, build_logs, ..
    }): State<AppState>,
    Path((owner, project, build_id)): Path<(String, String, Uuid)>,
) -> Response {
    let Some(user) = auth.current_user else {
        return StatusCode::UNAUTHORIZED.into_response();
    };

    let build = sqlx::query_as::<_, (String, String)>(
        r#"SELECT builds.status::text, builds.log
           FROM builds
           JOIN projects ON builds.project_id = projects.id
           JOIN project_owners ON projects.owner_id = project_owners.id
           LEFT JOIN users_owners ON project_owners.id = users_owners.owner_id
           LEFT JOIN project_shares ON projects.id = project_shares.project_id
           WHERE builds.id = $1
             AND projects.name = $2
             AND project_owners.name = $3
             AND (users_owners.user_id = $4 OR project_shares.user_id = $4)"#,
    )
    .bind(build_id)
    .bind(&project)
    .bind(&owner)
    .bind(user.id)
    .fetch_optional(&pool)
    .await;

    let (database_status, database_log) = match build {
        Ok(Some(build)) => build,
        Ok(None) => return StatusCode::NOT_FOUND.into_response(),
        Err(err) => {
            tracing::error!(?err, %build_id, "Failed to query build for SSE stream");
            return StatusCode::INTERNAL_SERVER_ERROR.into_response();
        }
    };

    // Subscribe before taking the snapshot. Events already represented by the
    // snapshot are skipped by sequence number, so no chunk can be lost here.
    let log_state = build_logs.read().await.get(&build_id).cloned();
    let (receiver, snapshot, snapshot_status, snapshot_queue_position, snapshot_sequence) =
        if let Some(state) = &log_state {
            let receiver = state.subscribe();
            let (snapshot, status, position, sequence) = state.snapshot().await;
            (Some(receiver), snapshot, status, position, sequence)
        } else {
            (None, database_log, database_status, None, 0)
        };

    let mut initial = VecDeque::new();
    initial.push_back(json_event(
        "snapshot",
        Snapshot {
            logs: &snapshot,
            sequence: snapshot_sequence,
        },
    ));
    initial.push_back(json_event(
        "status",
        Status {
            status: &snapshot_status,
            sequence: snapshot_sequence,
        },
    ));
    initial.push_back(json_event(
        "queue",
        QueuePosition {
            position: snapshot_queue_position,
            sequence: snapshot_sequence,
        },
    ));

    let output = stream::unfold(
        StreamState {
            initial,
            receiver,
            log_state,
            snapshot_sequence,
        },
        |mut state| async move {
            if let Some(event) = state.initial.pop_front() {
                return Some((Ok::<Event, Infallible>(event), state));
            }

            loop {
                let receiver = state.receiver.as_mut()?;
                match receiver.recv().await {
                    Ok(event) if event.sequence <= state.snapshot_sequence => continue,
                    Ok(event) => {
                        let sse = match event.event.as_str() {
                            "log" => json_event(
                                "log",
                                Chunk {
                                    chunk: &event.data,
                                    sequence: event.sequence,
                                },
                            ),
                            "queue" => json_event(
                                "queue",
                                QueuePosition {
                                    position: event.data.parse().ok(),
                                    sequence: event.sequence,
                                },
                            ),
                            _ => json_event(
                                "status",
                                Status {
                                    status: &event.data,
                                    sequence: event.sequence,
                                },
                            ),
                        };
                        state.snapshot_sequence = event.sequence;
                        return Some((Ok(sse), state));
                    }
                    Err(broadcast::error::RecvError::Lagged(_)) => {
                        if let Some(log_state) = &state.log_state {
                            let (snapshot, _, _, sequence) = log_state.snapshot().await;
                            state.snapshot_sequence = sequence;
                            let event = json_event(
                                "snapshot",
                                Snapshot {
                                    logs: &snapshot,
                                    sequence,
                                },
                            );
                            return Some((Ok(event), state));
                        }
                    }
                    Err(broadcast::error::RecvError::Closed) => return None,
                }
            }
        },
    );

    Sse::new(output)
        .keep_alive(
            KeepAlive::new()
                .interval(Duration::from_secs(15))
                .text("keep-alive"),
        )
        .into_response()
}
