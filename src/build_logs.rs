use std::{collections::HashMap, sync::{Arc, atomic::{AtomicU64, Ordering}}};

use serde::Serialize;
use tokio::sync::{broadcast, Mutex, RwLock};
use uuid::Uuid;

const MAX_BUILD_LOG_BYTES: usize = 10 * 1024 * 1024;

pub type BuildLogRegistry = Arc<RwLock<HashMap<Uuid, Arc<BuildLogState>>>>;

#[derive(Clone, Debug, Serialize)]
pub struct BuildLogEvent {
    pub sequence: u64,
    pub event: String,
    pub data: String,
}

#[derive(Debug)]
pub struct BuildLogState {
    snapshot: Mutex<BuildLogSnapshot>,
    sequence: AtomicU64,
    sender: broadcast::Sender<BuildLogEvent>,
}

#[derive(Debug)]
struct BuildLogSnapshot {
    logs: String,
    status: String,
    queue_position: Option<usize>,
}

impl BuildLogState {
    pub fn new() -> Arc<Self> {
        let (sender, _) = broadcast::channel(1024);
        Arc::new(Self {
            snapshot: Mutex::new(BuildLogSnapshot {
                logs: String::new(),
                status: "pending".to_string(),
                queue_position: None,
            }),
            sequence: AtomicU64::new(0),
            sender,
        })
    }

    pub fn subscribe(&self) -> broadcast::Receiver<BuildLogEvent> {
        self.sender.subscribe()
    }

    pub async fn snapshot(&self) -> (String, String, Option<usize>, u64) {
        let snapshot = self.snapshot.lock().await;
        (
            snapshot.logs.clone(),
            snapshot.status.clone(),
            snapshot.queue_position,
            self.sequence.load(Ordering::SeqCst),
        )
    }

    pub async fn append(&self, chunk: &str) {
        let mut snapshot = self.snapshot.lock().await;
        let remaining = MAX_BUILD_LOG_BYTES.saturating_sub(snapshot.logs.len());
        if remaining == 0 {
            return;
        }

        let mut end = chunk.len().min(remaining);
        while end > 0 && !chunk.is_char_boundary(end) {
            end -= 1;
        }
        if end == 0 {
            return;
        }

        let chunk = &chunk[..end];
        snapshot.logs.push_str(chunk);
        let sequence = self.sequence.fetch_add(1, Ordering::SeqCst) + 1;
        let _ = self.sender.send(BuildLogEvent {
            sequence,
            event: "log".to_string(),
            data: chunk.to_string(),
        });
    }

    pub async fn status(&self, status: &str) {
        let mut snapshot = self.snapshot.lock().await;
        snapshot.status = status.to_string();
        let sequence = self.sequence.fetch_add(1, Ordering::SeqCst) + 1;
        let _ = self.sender.send(BuildLogEvent {
            sequence,
            event: "status".to_string(),
            data: status.to_string(),
        });
    }

    pub async fn queue_position(&self, position: Option<usize>) {
        let mut snapshot = self.snapshot.lock().await;
        if snapshot.queue_position == position {
            return;
        }
        snapshot.queue_position = position;
        let sequence = self.sequence.fetch_add(1, Ordering::SeqCst) + 1;
        let _ = self.sender.send(BuildLogEvent {
            sequence,
            event: "queue".to_string(),
            data: position.map(|value| value.to_string()).unwrap_or_default(),
        });
    }
}

pub fn new_registry() -> BuildLogRegistry {
    Arc::new(RwLock::new(HashMap::new()))
}
