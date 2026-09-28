use parking_lot::Mutex;
use sockudo_core::options::Readiness;
use std::sync::Arc;

#[derive(Default)]
pub(super) struct Capacity {
    state: Mutex<State>,
}

#[derive(Default)]
struct State {
    live: usize,
    shedding: bool,
}

impl Capacity {
    pub(super) fn track(self: &Arc<Self>) -> LiveConnection {
        self.state.lock().live += 1;
        LiveConnection(Some(Arc::clone(self)))
    }

    pub(super) fn is_ready(&self, max: u32, config: &Readiness) -> bool {
        let mut state = self.state.lock();
        if max == 0 {
            state.shedding = false;
        } else {
            let load = state.live as f64 / f64::from(max);
            if load >= config.high_watermark {
                state.shedding = true;
            } else if load <= config.low_watermark {
                state.shedding = false;
            }
        }
        !state.shedding
    }
}

pub(super) struct LiveConnection(Option<Arc<Capacity>>);

impl LiveConnection {
    pub(super) fn release(&mut self) {
        if let Some(capacity) = self.0.take() {
            capacity.state.lock().live -= 1;
        }
    }
}

impl Drop for LiveConnection {
    fn drop(&mut self) {
        self.release();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn capacity_hysteresis_counts_live_connections_only() {
        let capacity = Arc::new(Capacity::default());
        let config = Readiness::default();
        let mut sockets: Vec<_> = (0..95).map(|_| capacity.track()).collect();
        assert!(!capacity.is_ready(100, &config));
        sockets.truncate(90);
        assert!(!capacity.is_ready(100, &config));
        // Cleanup may retain its own socket state after the live guard is released.
        for socket in &mut sockets[85..] {
            socket.release();
        }
        assert!(capacity.is_ready(100, &config));
        sockets.truncate(85); // Releasing twice must not decrement again.
        assert_eq!(capacity.state.lock().live, 85);
        sockets.extend((0..9).map(|_| capacity.track()));
        assert!(capacity.is_ready(100, &config));
        sockets.push(capacity.track());
        assert!(!capacity.is_ready(100, &config));
        assert!(capacity.is_ready(0, &config));
    }

    #[tokio::test]
    async fn aborted_socket_task_releases_capacity() {
        let capacity = Arc::new(Capacity::default());
        let guard = capacity.track();
        let task = tokio::spawn(async move {
            let _guard = guard;
            std::future::pending::<()>().await;
        });
        assert!(!capacity.is_ready(1, &Readiness::default()));
        task.abort();
        assert!(task.await.unwrap_err().is_cancelled());
        assert!(capacity.is_ready(1, &Readiness::default()));
    }
}
