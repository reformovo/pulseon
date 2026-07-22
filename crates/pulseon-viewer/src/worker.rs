use std::path::{Path, PathBuf};
use std::sync::mpsc::{self, Receiver, Sender};
use std::thread::{self, JoinHandle};
use std::time::Duration;

use crate::model::{CatalogSnapshot, DiscoveryRequest};
use crate::query::{CurveSnapshot, DetailRequest, OverviewRequest, QueryError};
use crate::source::{ReadSession, SourceError};

/// Monotonically increasing identity for one read request.
#[derive(Clone, Copy, Debug, Eq, Ord, PartialEq, PartialOrd)]
pub struct Generation(pub u64);

/// Independent result streams maintained by the viewer Core.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ReadKind {
    Catalog,
    Overview,
    Detail,
}

/// Storage work accepted by the native read worker.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum ReadRequest {
    Discover(DiscoveryRequest),
    Overview(OverviewRequest),
    Detail(DetailRequest),
}

impl ReadRequest {
    pub const fn kind(&self) -> ReadKind {
        match self {
            Self::Discover(_) => ReadKind::Catalog,
            Self::Overview(_) => ReadKind::Overview,
            Self::Detail(_) => ReadKind::Detail,
        }
    }
}

/// Immutable result payload returned by the worker.
#[derive(Clone, Debug, PartialEq)]
pub enum ReadSnapshot {
    Catalog(CatalogSnapshot),
    Overview(CurveSnapshot),
    Detail(CurveSnapshot),
}

impl ReadSnapshot {
    pub const fn kind(&self) -> ReadKind {
        match self {
            Self::Catalog(_) => ReadKind::Catalog,
            Self::Overview(_) => ReadKind::Overview,
            Self::Detail(_) => ReadKind::Detail,
        }
    }
}

/// Worker execution failures.
#[derive(Debug, thiserror::Error)]
pub enum WorkerError {
    #[error(transparent)]
    Query(Box<QueryError>),
    #[error(transparent)]
    Source(#[from] SourceError),
    #[error("native read session was not initialized")]
    SessionUnavailable,
}

impl From<QueryError> for WorkerError {
    fn from(error: QueryError) -> Self {
        Self::Query(Box::new(error))
    }
}

/// One generation-tagged result or failure.
#[derive(Debug)]
pub struct ReadEvent {
    pub generation: Generation,
    pub kind: ReadKind,
    pub result: Result<ReadSnapshot, WorkerError>,
}

/// Returned when the read worker no longer accepts requests.
#[derive(Clone, Copy, Debug, Eq, PartialEq, thiserror::Error)]
#[error("native read worker is closed")]
pub struct WorkerClosed;

struct TaggedRequest {
    generation: Generation,
    request: ReadRequest,
}

/// Handle for one background thread that exclusively owns its read session.
pub struct ReadWorker {
    requests: Option<Sender<TaggedRequest>>,
    events: Receiver<ReadEvent>,
    thread: Option<JoinHandle<()>>,
}

impl ReadWorker {
    /// Starts a worker for one local native source.
    ///
    /// # Errors
    ///
    /// Returns an I/O error when the operating system cannot spawn the thread.
    pub fn spawn(root_path: &Path) -> Result<Self, std::io::Error> {
        let root_path = root_path.to_path_buf();
        let (request_tx, request_rx) = mpsc::channel();
        let (event_tx, event_rx) = mpsc::channel();
        let thread = thread::Builder::new()
            .name("pulseon-native-reader".to_owned())
            .spawn(move || worker_loop(root_path, request_rx, event_tx))?;
        Ok(Self {
            requests: Some(request_tx),
            events: event_rx,
            thread: Some(thread),
        })
    }

    /// Queues storage work without waiting for its execution.
    ///
    /// # Errors
    ///
    /// Returns [`WorkerClosed`] after the worker has stopped.
    pub fn submit(&self, generation: Generation, request: ReadRequest) -> Result<(), WorkerClosed> {
        self.requests
            .as_ref()
            .ok_or(WorkerClosed)?
            .send(TaggedRequest {
                generation,
                request,
            })
            .map_err(|_| WorkerClosed)
    }

    pub fn try_event(&self) -> Option<ReadEvent> {
        self.events.try_recv().ok()
    }

    /// Waits for an event from background coordination or test code.
    ///
    /// # Errors
    ///
    /// Returns a receive error if the timeout expires or the worker stops.
    pub fn recv_timeout(&self, timeout: Duration) -> Result<ReadEvent, mpsc::RecvTimeoutError> {
        self.events.recv_timeout(timeout)
    }
}

impl Drop for ReadWorker {
    fn drop(&mut self) {
        self.requests.take();
        // A native query cannot currently be cancelled. Detach it so dropping
        // viewer state never blocks the UI thread while the query finishes.
        drop(self.thread.take());
    }
}

#[derive(Default)]
struct PendingRequests {
    discover: Option<TaggedRequest>,
    overview: Option<TaggedRequest>,
    detail: Option<TaggedRequest>,
}

impl PendingRequests {
    fn push(&mut self, tagged: TaggedRequest) {
        let slot = match &tagged.request {
            ReadRequest::Discover(_) => &mut self.discover,
            ReadRequest::Overview(_) => &mut self.overview,
            ReadRequest::Detail(_) => &mut self.detail,
        };
        if slot
            .as_ref()
            .is_none_or(|pending| tagged.generation >= pending.generation)
        {
            *slot = Some(tagged);
        }
    }

    fn take_next(&mut self) -> Option<TaggedRequest> {
        self.discover
            .take()
            .or_else(|| self.overview.take())
            .or_else(|| self.detail.take())
    }
}

fn worker_loop(root_path: PathBuf, requests: Receiver<TaggedRequest>, events: Sender<ReadEvent>) {
    let mut session = None;
    while let Ok(first) = requests.recv() {
        let mut pending = PendingRequests::default();
        pending.push(first);
        loop {
            for request in requests.try_iter() {
                pending.push(request);
            }
            let Some(request) = pending.take_next() else {
                break;
            };
            let event = execute(&root_path, &mut session, request);
            if events.send(event).is_err() {
                return;
            }
        }
    }
}

fn execute(
    root_path: &Path,
    session: &mut Option<ReadSession>,
    tagged: TaggedRequest,
) -> ReadEvent {
    let kind = tagged.request.kind();
    let result = (|| {
        if session.is_none() {
            *session = Some(ReadSession::open_existing(root_path)?);
        }
        let session = session.as_ref().ok_or(WorkerError::SessionUnavailable)?;
        Ok(match tagged.request {
            ReadRequest::Discover(request) => ReadSnapshot::Catalog(session.discover(&request)?),
            ReadRequest::Overview(request) => {
                ReadSnapshot::Overview(session.query_overview(&request)?)
            }
            ReadRequest::Detail(request) => ReadSnapshot::Detail(session.query_detail(&request)?),
        })
    })();
    ReadEvent {
        generation: tagged.generation,
        kind,
        result,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn dropping_worker_does_not_wait_for_running_thread() {
        let (request_tx, _request_rx) = mpsc::channel();
        let (_event_tx, event_rx) = mpsc::channel();
        let (release_tx, release_rx) = mpsc::channel();
        let (finished_tx, finished_rx) = mpsc::channel();
        let thread = thread::spawn(move || {
            let _ = release_rx.recv();
            let _ = finished_tx.send(());
        });
        let worker = ReadWorker {
            requests: Some(request_tx),
            events: event_rx,
            thread: Some(thread),
        };
        let (dropped_tx, dropped_rx) = mpsc::channel();
        let dropper = thread::spawn(move || {
            drop(worker);
            let _ = dropped_tx.send(());
        });

        let dropped = dropped_rx.recv_timeout(Duration::from_secs(1));
        release_tx.send(()).expect("test worker should still exist");
        finished_rx
            .recv_timeout(Duration::from_secs(1))
            .expect("test worker should finish after release");
        dropper.join().expect("dropper should not panic");

        assert!(dropped.is_ok(), "worker drop waited for its running thread");
    }

    #[test]
    fn pending_requests_keep_the_latest_generation_per_kind() {
        let mut pending = PendingRequests::default();
        for generation in [1, 2] {
            pending.push(TaggedRequest {
                generation: Generation(generation),
                request: ReadRequest::Discover(DiscoveryRequest::default()),
            });
        }

        assert_eq!(
            pending.take_next().map(|request| request.generation),
            Some(Generation(2))
        );
    }
}
