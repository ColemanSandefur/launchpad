use serde::{Deserialize, Serialize};
use std::collections::VecDeque;
use std::sync::Arc;
use tokio::sync::Mutex;

#[derive(Debug, Clone)]
struct BufferState {
    deque: VecDeque<String>,
    total_pushed: usize, // Monotonically increasing counter of all lines ever added
}

#[derive(Debug, Clone)]
pub struct LogBuffer {
    state: Arc<Mutex<BufferState>>,
    max_capacity: usize,
}

impl LogBuffer {
    pub fn new(max_capacity: usize) -> Self {
        Self {
            state: Arc::new(Mutex::new(BufferState {
                deque: VecDeque::with_capacity(max_capacity),
                total_pushed: 0,
            })),
            max_capacity,
        }
    }

    pub async fn push(&self, line: String) {
        let mut state = self.state.lock().await;
        if state.deque.len() >= self.max_capacity {
            state.deque.pop_front(); // Drops the oldest line
        }
        state.deque.push_back(line);
        state.total_pushed += 1;
    }

    /// Fetches lines starting from a client's requested sequence number.
    /// Returns: (Vec of new lines, next sequence number to poll, whether lines were dropped)
    pub async fn get_since(&self, client_seq: usize) -> LogQueryResult {
        let state = self.state.lock().await;

        let total_pushed = state.total_pushed;
        let len = state.deque.len();

        // The absolute sequence number of the oldest item currently in the buffer
        let oldest_seq = total_pushed.saturating_sub(len);

        let mut lines = Vec::new();
        let mut dropped_history = false;

        if client_seq < oldest_seq {
            // The client is asking for lines that have already been evicted from the ring buffer!
            // You can either error out or reset them to the oldest available.
            dropped_history = true;
        }

        // Calculate how many items to skip from the front of our deque
        let skip_count = if client_seq < oldest_seq {
            0 // Fallback to start of current buffer
        } else {
            client_seq - oldest_seq
        };

        if skip_count < len {
            lines = state.deque.iter().skip(skip_count).cloned().collect();
        }

        LogQueryResult {
            lines,
            next_idx: total_pushed,
            dropped_history,
        }
    }
}

#[derive(Serialize, Deserialize, Debug, PartialEq, Eq)]
pub struct LogQueryResult {
    pub lines: Vec<String>,
    pub next_idx: usize,
    pub dropped_history: bool,
}
