//! Michelangelo Jobs — job scheduler and event queue.
//!
//! Provides an in-process job runner with queue, lifecycle state machine,
//! and event emission for headless job execution.
//!
//! Phase 2 supports max concurrency `1` for Blender jobs.

use std::collections::VecDeque;
use std::sync::mpsc::{self, Receiver, Sender};
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use michelangelo_protocol::event::{
    EventEnvelope, JOB_CANCELLED, JOB_COMPLETED, JOB_FAILED, JOB_PROGRESS, JOB_QUEUED, JOB_STARTED,
};
use michelangelo_protocol::job::{ArtifactDto, JobStatus};

// ---------------------------------------------------------------------------
// Data types
// ---------------------------------------------------------------------------

/// Unique identifier for a queued job.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct JobId(pub String);

/// Internal job state tracked by the scheduler.
#[derive(Debug, Clone)]
pub struct JobState {
    pub id: JobId,
    pub project_id: String,
    pub job_type: String,
    pub status: JobStatus,
    pub progress_pct: Option<u8>,
    pub message: Option<String>,
    pub created_at: String,
    pub started_at: Option<String>,
    pub finished_at: Option<String>,
    pub error: Option<String>,
    pub artifacts: Vec<ArtifactDto>,
}

// ---------------------------------------------------------------------------
// Event types
// ---------------------------------------------------------------------------

/// A typed event emitted by the scheduler.
#[derive(Debug, Clone)]
pub enum JobEvent {
    Queued {
        job_id: String,
        job_type: String,
    },
    Started {
        job_id: String,
        job_type: String,
    },
    Progress {
        job_id: String,
        job_type: String,
        progress_pct: u8,
        message: Option<String>,
    },
    Completed {
        job_id: String,
        job_type: String,
        artifacts: Vec<ArtifactDto>,
    },
    Failed {
        job_id: String,
        job_type: String,
        error: String,
    },
    Cancelled {
        job_id: String,
        job_type: String,
    },
}

impl JobEvent {
    /// Convert to protocol EventEnvelope.
    pub fn to_envelope(&self) -> EventEnvelope {
        match self {
            JobEvent::Queued { job_id, job_type } => EventEnvelope::new(
                JOB_QUEUED,
                serde_json::json!({
                    "job_id": job_id,
                    "job_type": job_type,
                }),
            ),
            JobEvent::Started { job_id, job_type } => EventEnvelope::new(
                JOB_STARTED,
                serde_json::json!({
                    "job_id": job_id,
                    "job_type": job_type,
                }),
            ),
            JobEvent::Progress {
                job_id,
                job_type,
                progress_pct,
                message,
            } => EventEnvelope::new(
                JOB_PROGRESS,
                serde_json::json!({
                    "job_id": job_id,
                    "job_type": job_type,
                    "progress_pct": progress_pct,
                    "message": message,
                }),
            ),
            JobEvent::Completed {
                job_id,
                job_type,
                artifacts,
            } => EventEnvelope::new(
                JOB_COMPLETED,
                serde_json::json!({
                    "job_id": job_id,
                    "job_type": job_type,
                    "artifacts": artifacts,
                }),
            ),
            JobEvent::Failed {
                job_id,
                job_type,
                error,
            } => EventEnvelope::new(
                JOB_FAILED,
                serde_json::json!({
                    "job_id": job_id,
                    "job_type": job_type,
                    "error": error,
                }),
            ),
            JobEvent::Cancelled { job_id, job_type } => EventEnvelope::new(
                JOB_CANCELLED,
                serde_json::json!({
                    "job_id": job_id,
                    "job_type": job_type,
                }),
            ),
        }
    }
}

// ---------------------------------------------------------------------------
// Scheduler
// ---------------------------------------------------------------------------

/// In-process job scheduler with max concurrency 1.
///
/// The scheduler maintains a FIFO queue of jobs, a single running slot,
/// and an event channel for lifecycle notifications.
pub struct Scheduler {
    queue: VecDeque<JobState>,
    running: Option<JobState>,
    completed: Vec<JobState>,
    event_tx: Sender<JobEvent>,
    next_id: u64,
}

impl Scheduler {
    /// Create a new scheduler with an event channel.
    ///
    /// Returns the scheduler and a `Receiver` for consuming `JobEvent`s.
    pub fn new() -> (Self, Receiver<JobEvent>) {
        let (tx, rx) = mpsc::channel();
        let scheduler = Scheduler {
            queue: VecDeque::new(),
            running: None,
            completed: Vec::new(),
            event_tx: tx,
            next_id: 1,
        };
        (scheduler, rx)
    }

    /// Enqueue a new job.
    ///
    /// Returns the assigned job id.
    /// Fails if a job of the same type is already running.
    pub fn enqueue(&mut self, project_id: &str, job_type: &str) -> Result<JobId, String> {
        if let Some(ref running) = self.running {
            if running.job_type == job_type {
                return Err(format!("a job of type '{}' is already running", job_type));
            }
        }

        let id = format!("job-{}", self.next_id);
        self.next_id += 1;

        let job = JobState {
            id: JobId(id.clone()),
            project_id: project_id.to_string(),
            job_type: job_type.to_string(),
            status: JobStatus::Queued,
            progress_pct: None,
            message: None,
            created_at: now_iso8601(),
            started_at: None,
            finished_at: None,
            error: None,
            artifacts: Vec::new(),
        };

        self.queue.push_back(job);

        self.send_event(JobEvent::Queued {
            job_id: id.clone(),
            job_type: job_type.to_string(),
        })?;

        Ok(JobId(id))
    }

    /// Get next queued job (transition queued→running).
    ///
    /// Returns `None` if nothing is queued or max concurrency is reached.
    pub fn dequeue(&mut self) -> Option<JobState> {
        if !self.can_run() {
            return None;
        }
        let mut job = self.queue.pop_front()?;
        job.status = JobStatus::Running;
        job.started_at = Some(now_iso8601());

        let job_clone = job.clone();
        self.running = Some(job);

        // Ignore send error — if the receiver is dropped we still move the job.
        let _ = self.event_tx.send(JobEvent::Started {
            job_id: job_clone.id.0.clone(),
            job_type: job_clone.job_type.clone(),
        });

        Some(job_clone)
    }

    /// Advance the running job to completed.
    pub fn complete(&mut self, job_id: &str, artifacts: Vec<ArtifactDto>) -> Result<(), String> {
        let mut job = self
            .running
            .take()
            .ok_or_else(|| "no job is running".to_string())?;
        if job.id.0 != job_id {
            self.running = Some(job);
            return Err(format!("job {} is not running", job_id));
        }
        job.status = JobStatus::Completed;
        job.artifacts = artifacts;
        job.finished_at = Some(now_iso8601());

        let event = JobEvent::Completed {
            job_id: job.id.0.clone(),
            job_type: job.job_type.clone(),
            artifacts: job.artifacts.clone(),
        };

        self.completed.push(job);
        self.send_event(event)?;
        Ok(())
    }

    /// Mark the running job as failed.
    pub fn fail(&mut self, job_id: &str, error: &str) -> Result<(), String> {
        let mut job = self
            .running
            .take()
            .ok_or_else(|| "no job is running".to_string())?;
        if job.id.0 != job_id {
            self.running = Some(job);
            return Err(format!("job {} is not running", job_id));
        }
        job.status = JobStatus::Failed;
        job.error = Some(error.to_string());
        job.finished_at = Some(now_iso8601());

        let event = JobEvent::Failed {
            job_id: job.id.0.clone(),
            job_type: job.job_type.clone(),
            error: error.to_string(),
        };

        self.completed.push(job);
        self.send_event(event)?;
        Ok(())
    }

    /// Cancel a job (queued immediately, running best-effort).
    pub fn cancel(&mut self, job_id: &str) -> Result<(), String> {
        // Check queued.
        if let Some(pos) = self.queue.iter().position(|j| j.id.0 == job_id) {
            let mut job = self.queue.remove(pos).unwrap();
            job.status = JobStatus::Cancelled;
            job.finished_at = Some(now_iso8601());

            let event = JobEvent::Cancelled {
                job_id: job.id.0.clone(),
                job_type: job.job_type.clone(),
            };

            self.completed.push(job);
            return self.send_event(event);
        }

        // Check running.
        if let Some(ref mut job) = self.running {
            if job.id.0 == job_id {
                job.status = JobStatus::Cancelled;
                job.finished_at = Some(now_iso8601());

                let event = JobEvent::Cancelled {
                    job_id: job.id.0.clone(),
                    job_type: job.job_type.clone(),
                };

                let cancelled = self.running.take().unwrap();
                self.completed.push(cancelled);
                return self.send_event(event);
            }
        }

        Err(format!("job {} not found", job_id))
    }

    /// Update progress for the running job.
    pub fn update_progress(
        &mut self,
        job_id: &str,
        pct: u8,
        message: Option<&str>,
    ) -> Result<(), String> {
        let pct = pct.min(100);
        let job_type = self
            .running
            .as_ref()
            .filter(|j| j.id.0 == job_id)
            .map(|j| j.job_type.clone())
            .ok_or_else(|| format!("job {} is not running", job_id))?;

        if let Some(ref mut job) = self.running {
            if job.id.0 == job_id {
                job.progress_pct = Some(pct);
                job.message = message.map(String::from);
            }
        }

        self.send_event(JobEvent::Progress {
            job_id: job_id.to_string(),
            job_type,
            progress_pct: pct,
            message: message.map(String::from),
        })?;
        Ok(())
    }

    /// List all jobs (queued, running, and completed).
    pub fn list_jobs(&self) -> Vec<&JobState> {
        self.queue
            .iter()
            .chain(self.running.iter())
            .chain(self.completed.iter())
            .collect()
    }

    /// Get a specific job by id.
    pub fn get_job(&self, job_id: &str) -> Option<&JobState> {
        self.queue
            .iter()
            .chain(self.running.iter())
            .chain(self.completed.iter())
            .find(|j| j.id.0 == job_id)
    }

    /// Check if a new job can be started (max concurrency = 1).
    pub fn can_run(&self) -> bool {
        self.running.is_none()
    }

    /// Number of queued jobs.
    pub fn queued_count(&self) -> usize {
        self.queue.len()
    }

    /// Wait for a job to reach terminal state, with optional timeout.
    ///
    /// Polls internal state; returns the terminal `JobState` or an error
    /// if the timeout expires or the job is not found.
    pub fn wait_for_job(
        &mut self,
        job_id: &str,
        timeout_ms: Option<u64>,
    ) -> Result<JobState, String> {
        // Pre-check existence.
        let exists = self
            .queue
            .iter()
            .chain(self.running.iter())
            .chain(self.completed.iter())
            .any(|j| j.id.0 == job_id);

        if !exists {
            return Err(format!("job {} not found", job_id));
        }

        let start = Instant::now();
        loop {
            if let Some(ms) = timeout_ms {
                if start.elapsed().as_millis() as u64 >= ms {
                    return Err(format!("timeout waiting for job {}", job_id));
                }
            }

            let terminal = self
                .queue
                .iter()
                .chain(self.running.iter())
                .chain(self.completed.iter())
                .find(|j| j.id.0 == job_id)
                .cloned();

            match terminal {
                Some(job) if job.status.is_terminal() => return Ok(job),
                Some(_) => {}
                None => return Err(format!("job {} not found", job_id)),
            }

            std::thread::sleep(Duration::from_millis(10));
        }
    }

    // -- internal helpers ---------------------------------------------------

    fn send_event(&self, event: JobEvent) -> Result<(), String> {
        self.event_tx
            .send(event)
            .map_err(|_| "event channel closed".to_string())
    }
}

// ---------------------------------------------------------------------------
// Timestamp helper
// ---------------------------------------------------------------------------

fn now_iso8601() -> String {
    let duration = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default();
    let total_secs = duration.as_secs();

    let days = total_secs / 86400;
    let time_secs = total_secs % 86400;
    let hours = time_secs / 3600;
    let minutes = (time_secs % 3600) / 60;
    let seconds = time_secs % 60;

    let mut y = 1970i64;
    let mut remaining = days as i64;
    loop {
        let days_in_year = if is_leap(y) { 366 } else { 365 };
        if remaining < days_in_year {
            break;
        }
        remaining -= days_in_year;
        y += 1;
    }

    let month_days = if is_leap(y) {
        [31, 29, 31, 30, 31, 30, 31, 31, 30, 31, 30, 31]
    } else {
        [31, 28, 31, 30, 31, 30, 31, 31, 30, 31, 30, 31]
    };

    let mut m = 1usize;
    let mut remaining_local = remaining;
    for &md in &month_days {
        if remaining_local < md {
            break;
        }
        remaining_local -= md;
        m += 1;
    }
    let d = remaining_local + 1;

    format!("{y:04}-{m:02}-{d:02}T{hours:02}:{minutes:02}:{seconds:02}Z")
}

fn is_leap(year: i64) -> bool {
    (year % 4 == 0 && year % 100 != 0) || (year % 400 == 0)
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    // -- lifecycle ----------------------------------------------------------

    #[test]
    fn test_scheduler_new_is_empty() {
        let (scheduler, _rx) = Scheduler::new();
        assert!(scheduler.list_jobs().is_empty());
        assert_eq!(scheduler.queued_count(), 0);
        assert!(scheduler.can_run());
        assert!(scheduler.running.is_none());
    }

    #[test]
    fn test_enqueue_increases_count() {
        let (mut scheduler, _rx) = Scheduler::new();
        scheduler.enqueue("proj1", "blender_render").unwrap();
        assert_eq!(scheduler.queued_count(), 1);
        assert_eq!(scheduler.list_jobs().len(), 1);
    }

    #[test]
    fn test_dequeue_transitions_to_running() {
        let (mut scheduler, _rx) = Scheduler::new();
        scheduler.enqueue("proj1", "blender_render").unwrap();
        let state = scheduler.dequeue().unwrap();
        assert_eq!(state.status, JobStatus::Running);
        assert_eq!(scheduler.queued_count(), 0);
        assert!(!scheduler.can_run());
    }

    #[test]
    fn test_complete_transitions_and_emits_event() {
        let (mut scheduler, rx) = Scheduler::new();
        let job_id = scheduler.enqueue("proj1", "render").unwrap();
        scheduler.dequeue();
        scheduler.complete(&job_id.0, vec![]).unwrap();

        let job = scheduler.get_job(&job_id.0).unwrap();
        assert_eq!(job.status, JobStatus::Completed);
        assert!(scheduler.can_run());

        let events: Vec<JobEvent> = rx.try_iter().collect();
        assert!(events
            .iter()
            .any(|e| matches!(e, JobEvent::Completed { .. })));
    }

    #[test]
    fn test_fail_transitions_and_emits_event() {
        let (mut scheduler, rx) = Scheduler::new();
        let job_id = scheduler.enqueue("proj1", "render").unwrap();
        scheduler.dequeue();
        scheduler.fail(&job_id.0, "something went wrong").unwrap();

        let job = scheduler.get_job(&job_id.0).unwrap();
        assert_eq!(job.status, JobStatus::Failed);
        assert_eq!(job.error.as_deref(), Some("something went wrong"));
        assert!(scheduler.can_run());

        let events: Vec<JobEvent> = rx.try_iter().collect();
        assert!(events.iter().any(|e| matches!(e, JobEvent::Failed { .. })));
    }

    #[test]
    fn test_cancel_queued_job() {
        let (mut scheduler, rx) = Scheduler::new();
        let job_id = scheduler.enqueue("proj1", "render").unwrap();
        scheduler.cancel(&job_id.0).unwrap();

        // Job moved from queue to completed with Cancelled status.
        assert_eq!(scheduler.queued_count(), 0);
        let job = scheduler.get_job(&job_id.0).unwrap();
        assert_eq!(job.status, JobStatus::Cancelled);

        let events: Vec<JobEvent> = rx.try_iter().collect();
        assert!(events
            .iter()
            .any(|e| matches!(e, JobEvent::Cancelled { .. })));
    }

    #[test]
    fn test_cancel_running_job() {
        let (mut scheduler, rx) = Scheduler::new();
        let job_id = scheduler.enqueue("proj1", "render").unwrap();
        scheduler.dequeue();
        scheduler.cancel(&job_id.0).unwrap();

        // Running job moved to completed with Cancelled, slot is free.
        assert!(scheduler.can_run());
        let job = scheduler.get_job(&job_id.0).unwrap();
        assert_eq!(job.status, JobStatus::Cancelled);

        let events: Vec<JobEvent> = rx.try_iter().collect();
        assert!(events
            .iter()
            .any(|e| matches!(e, JobEvent::Cancelled { .. })));
    }

    #[test]
    fn test_cancel_unknown_job_returns_error() {
        let (mut scheduler, _rx) = Scheduler::new();
        let result = scheduler.cancel("nonexistent");
        assert!(result.is_err());
        assert!(result.unwrap_err().contains("not found"));
    }

    #[test]
    fn test_max_concurrency_one() {
        let (mut scheduler, _rx) = Scheduler::new();
        scheduler.enqueue("p1", "render").unwrap();
        scheduler.enqueue("p1", "export").unwrap();
        assert_eq!(scheduler.queued_count(), 2);

        let first = scheduler.dequeue().unwrap();
        assert_eq!(first.status, JobStatus::Running);
        assert!(!scheduler.can_run());

        // Second dequeue should return None (concurrency limit).
        assert!(scheduler.dequeue().is_none());
        assert_eq!(scheduler.queued_count(), 1);
    }

    #[test]
    fn test_progress_update() {
        let (mut scheduler, rx) = Scheduler::new();
        let job_id = scheduler.enqueue("p1", "render").unwrap();
        scheduler.dequeue();
        scheduler
            .update_progress(&job_id.0, 50, Some("processing frame 42"))
            .unwrap();

        let job = scheduler.get_job(&job_id.0).unwrap();
        assert_eq!(job.progress_pct, Some(50));
        assert_eq!(job.message.as_deref(), Some("processing frame 42"));

        let events: Vec<JobEvent> = rx.try_iter().collect();
        assert!(events
            .iter()
            .any(|e| matches!(e, JobEvent::Progress { .. })));
    }

    #[test]
    fn test_list_jobs_returns_all() {
        let (mut scheduler, _rx) = Scheduler::new();
        let j1 = scheduler.enqueue("p1", "render").unwrap();
        let j2 = scheduler.enqueue("p1", "export").unwrap();

        assert_eq!(scheduler.list_jobs().len(), 2);

        scheduler.dequeue();
        scheduler.complete(&j1.0, vec![]).unwrap();

        // Should still list both (one completed, one queued).
        assert_eq!(scheduler.list_jobs().len(), 2);
        assert_eq!(scheduler.queued_count(), 1);

        scheduler.dequeue();
        assert_eq!(scheduler.list_jobs().len(), 2);
        scheduler.complete(&j2.0, vec![]).unwrap();
        assert_eq!(scheduler.list_jobs().len(), 2);
    }

    #[test]
    fn test_get_job_by_id() {
        let (mut scheduler, _rx) = Scheduler::new();
        let job_id = scheduler.enqueue("p1", "render").unwrap();

        let found = scheduler.get_job(&job_id.0);
        assert!(found.is_some());
        assert_eq!(found.unwrap().id.0, job_id.0);

        assert!(scheduler.get_job("nonexistent").is_none());
    }

    #[test]
    fn test_wait_for_job_timeout() {
        let (mut scheduler, _rx) = Scheduler::new();
        let job_id = scheduler.enqueue("p1", "render").unwrap();

        // Job stays queued and never reaches terminal state → timeout.
        let result = scheduler.wait_for_job(&job_id.0, Some(10));
        assert!(result.is_err());
        assert!(result.unwrap_err().contains("timeout"));
    }

    #[test]
    fn test_complete_twice_returns_error() {
        let (mut scheduler, _rx) = Scheduler::new();
        let job_id = scheduler.enqueue("p1", "render").unwrap();
        scheduler.dequeue();
        scheduler.complete(&job_id.0, vec![]).unwrap();

        // Second complete should fail — no running job left.
        let result = scheduler.complete(&job_id.0, vec![]);
        assert!(result.is_err());
    }

    #[test]
    fn test_event_sequence() {
        let (mut scheduler, rx) = Scheduler::new();
        let job_id = scheduler.enqueue("p1", "render").unwrap();
        scheduler.dequeue();
        scheduler.complete(&job_id.0, vec![]).unwrap();

        let events: Vec<JobEvent> = rx.try_iter().collect();
        assert_eq!(events.len(), 3);

        // Check order.
        assert!(matches!(events[0], JobEvent::Queued { .. }));
        assert!(matches!(events[1], JobEvent::Started { .. }));
        assert!(matches!(events[2], JobEvent::Completed { .. }));
    }

    // -- conversion tests ---------------------------------------------------

    #[test]
    fn test_events_convert_to_protocol_envelopes() {
        let events = vec![
            JobEvent::Queued {
                job_id: "j1".into(),
                job_type: "render".into(),
            },
            JobEvent::Started {
                job_id: "j1".into(),
                job_type: "render".into(),
            },
            JobEvent::Progress {
                job_id: "j1".into(),
                job_type: "render".into(),
                progress_pct: 42,
                message: Some("progressing".into()),
            },
            JobEvent::Completed {
                job_id: "j1".into(),
                job_type: "render".into(),
                artifacts: vec![ArtifactDto::new("out.png", "png")],
            },
            JobEvent::Failed {
                job_id: "j1".into(),
                job_type: "render".into(),
                error: "oops".into(),
            },
            JobEvent::Cancelled {
                job_id: "j1".into(),
                job_type: "render".into(),
            },
        ];

        let names = [
            JOB_QUEUED,
            JOB_STARTED,
            JOB_PROGRESS,
            JOB_COMPLETED,
            JOB_FAILED,
            JOB_CANCELLED,
        ];

        for (event, expected_name) in events.iter().zip(names.iter()) {
            let envelope = event.to_envelope();
            assert_eq!(&envelope.event, expected_name);
            assert_eq!(envelope.data["job_id"], "j1");
        }

        // Verify envelope round-trips through JSON.
        for event in &events {
            let envelope = event.to_envelope();
            let json = serde_json::to_string(&envelope).unwrap();
            let roundtrip: EventEnvelope = serde_json::from_str(&json).unwrap();
            assert_eq!(roundtrip.event, envelope.event);
            assert_eq!(roundtrip.data["job_id"], envelope.data["job_id"]);
        }
    }
}
