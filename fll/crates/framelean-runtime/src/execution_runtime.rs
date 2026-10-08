use std::collections::HashMap;
use std::sync::mpsc::{self, Receiver, Sender};
use std::sync::{Arc, Condvar, Mutex};
use std::thread;
use std::time::{Duration, Instant};

use framelean_core::{EngineError, EngineErrorCode, ErrorKind, Result, TaskId};
use framelean_decision::ResolvedConfiguration;
use framelean_pipeline::MediaPipelinePlan;

use crate::{
    ExecutionBackend, ExecutionBackendControl, ExecutionBackendObserver, ExecutionBackendOutcome,
    ExecutionBackendRequest, ExecutionCheckpoint, ExecutionLaneControl, ExecutionLaneSnapshot,
    ExecutionOutputRequest, ExecutionPauseReason, ExecutionProgress, ExecutionResourcePool,
    ExecutionScheduler, ExecutionServices, ExecutionTaskState, OutputTransaction,
    ScheduledExecution,
};

const SAFE_PAUSE_TIMEOUT: Duration = Duration::from_secs(5);
const PROGRESS_EVENT_INTERVAL: Duration = Duration::from_millis(100);

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ExecutionRuntimePlan {
    pub source_path: std::path::PathBuf,
    pub output: ExecutionOutputRequest,
    pub pipeline: MediaPipelinePlan,
    pub configuration: ResolvedConfiguration,
    pub resource_pool: ExecutionResourcePool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ExecutionRuntimeEvent {
    pub execution_id: TaskId,
    pub resource_pool: ExecutionResourcePool,
    pub sequence: u64,
    pub state: ExecutionTaskState,
    pub pause_reason: Option<ExecutionPauseReason>,
    pub preempted_by_execution_id: Option<TaskId>,
    pub resume_depth: usize,
    pub progress: Option<ExecutionProgress>,
    pub output_path: Option<std::path::PathBuf>,
    pub error_code: Option<EngineErrorCode>,
    pub message: Option<String>,
}

struct RuntimeExecutionTask {
    plan: ExecutionRuntimePlan,
    state: ExecutionTaskState,
    control: Option<Arc<TaskControl>>,
    terminal_reported: bool,
}

#[derive(Default)]
struct TaskControlState {
    pause_requested: bool,
    paused: bool,
    cancel_requested: bool,
    terminal: bool,
    latest_progress: ExecutionProgress,
}

#[derive(Default)]
struct TaskControl {
    state: Mutex<TaskControlState>,
    changed: Condvar,
}

enum ThreadEventKind {
    State(ExecutionTaskState),
    Started,
    Progress(ExecutionProgress),
    Paused(ExecutionProgress),
    Resumed,
    Completed {
        progress: ExecutionProgress,
        output_path: std::path::PathBuf,
    },
    Cancelled(ExecutionProgress),
    Failed {
        code: EngineErrorCode,
        message: String,
    },
}

struct ThreadEvent {
    execution_id: TaskId,
    kind: ThreadEventKind,
}

pub struct ExecutionRuntime {
    services: ExecutionServices,
    scheduler: ExecutionScheduler,
    tasks: HashMap<TaskId, RuntimeExecutionTask>,
    user_paused: HashMap<TaskId, ScheduledExecution>,
    event_tx: Sender<ThreadEvent>,
    event_rx: Receiver<ThreadEvent>,
    next_execution_number: u64,
    next_event_sequence: u64,
}

impl ExecutionRuntime {
    pub fn new(services: ExecutionServices) -> Self {
        let (event_tx, event_rx) = mpsc::channel();
        Self {
            services,
            scheduler: ExecutionScheduler::new(),
            tasks: HashMap::new(),
            user_paused: HashMap::new(),
            event_tx,
            event_rx,
            next_execution_number: 1,
            next_event_sequence: 1,
        }
    }

    pub fn submit(&mut self, plan: ExecutionRuntimePlan) -> Result<TaskId> {
        let execution_id = TaskId::new(format!("execution-{}", self.next_execution_number))?;
        self.next_execution_number = self.next_execution_number.saturating_add(1);
        let resource_pool = plan.resource_pool;
        self.tasks.insert(
            execution_id.clone(),
            RuntimeExecutionTask {
                plan,
                state: ExecutionTaskState::Queued,
                control: None,
                terminal_reported: false,
            },
        );
        self.scheduler
            .enqueue(execution_id.clone(), resource_pool)?;
        self.advance_available_slots();
        self.sync_scheduled_states();
        Ok(execution_id)
    }

    pub fn preempt_and_start(&mut self, execution_id: &TaskId) -> Result<()> {
        let victims = self.scheduler.preemption_victims_for(execution_id)?;
        for victim_id in &victims {
            self.send_state_event(victim_id, ExecutionTaskState::Preempting)?;
        }
        let mut scheduler = std::mem::take(&mut self.scheduler);
        let mut control = RuntimeLaneControl {
            backend: Arc::clone(&self.services.backend),
            tasks: &mut self.tasks,
            event_tx: self.event_tx.clone(),
        };
        let result = scheduler.preempt_and_start(execution_id, &mut control);
        self.scheduler = scheduler;
        if result.is_ok() {
            self.sync_scheduled_states();
        } else {
            for victim_id in &victims {
                self.send_state_event(victim_id, ExecutionTaskState::Running)?;
            }
        }
        result
    }

    pub fn pause_for_user(&mut self, execution_id: &TaskId) -> Result<()> {
        if !self
            .scheduler
            .snapshot()
            .active_executions
            .iter()
            .any(|entry| entry.execution_id == *execution_id)
        {
            return Err(EngineError::invalid_task_state(
                "only an active execution can be paused",
            ));
        }
        self.send_state_event(execution_id, ExecutionTaskState::PauseRequested)?;
        let mut scheduler = std::mem::take(&mut self.scheduler);
        let mut control = RuntimeLaneControl {
            backend: Arc::clone(&self.services.backend),
            tasks: &mut self.tasks,
            event_tx: self.event_tx.clone(),
        };
        let result = scheduler.pause_active_for_user(execution_id, &mut control);
        self.scheduler = scheduler;
        let paused = match result {
            Ok(paused) => paused,
            Err(error) => {
                self.send_state_event(execution_id, ExecutionTaskState::Running)?;
                return Err(error);
            }
        };
        self.user_paused
            .insert(paused.execution_id.clone(), paused.clone());
        if let Some(task) = self.tasks.get_mut(execution_id) {
            task.state = ExecutionTaskState::Paused;
        }
        self.advance_available_slots();
        Ok(())
    }

    pub fn resume_user_paused(&mut self, execution_id: &TaskId) -> Result<()> {
        let paused = self.user_paused.remove(execution_id).ok_or_else(|| {
            EngineError::invalid_task_state("execution is not paused by the user")
        })?;
        let mut scheduler = std::mem::take(&mut self.scheduler);
        let mut control = RuntimeLaneControl {
            backend: Arc::clone(&self.services.backend),
            tasks: &mut self.tasks,
            event_tx: self.event_tx.clone(),
        };
        let result = scheduler.resume_user_paused(paused.clone(), &mut control);
        self.scheduler = scheduler;
        if let Err(error) = result {
            self.user_paused.insert(execution_id.clone(), paused);
            return Err(error);
        }
        self.sync_scheduled_states();
        Ok(())
    }

    pub fn cancel(&mut self, execution_id: &TaskId) -> Result<()> {
        let task = self
            .tasks
            .get_mut(execution_id)
            .ok_or_else(|| EngineError::invalid_argument("execution does not exist"))?;
        if task.state == ExecutionTaskState::Queued {
            self.scheduler
                .remove_non_active(execution_id)
                .ok_or_else(|| {
                    EngineError::invalid_task_state(
                        "queued execution is missing from the scheduler",
                    )
                })?;
            task.state = ExecutionTaskState::Cancelled;
            self.event_tx
                .send(ThreadEvent {
                    execution_id: execution_id.clone(),
                    kind: ThreadEventKind::Cancelled(ExecutionProgress::default()),
                })
                .map_err(|_| {
                    EngineError::new(ErrorKind::Runtime, "execution event channel is unavailable")
                })?;
            return Ok(());
        }
        let control = task.control.as_ref().ok_or_else(|| {
            EngineError::invalid_task_state("started execution has no control channel")
        })?;
        let mut state = control_state(control)?;
        if state.terminal {
            return Err(EngineError::invalid_task_state(
                "terminal execution cannot be cancelled",
            ));
        }
        self.user_paused.remove(execution_id);
        self.scheduler.remove_non_active(execution_id);
        state.cancel_requested = true;
        task.state = ExecutionTaskState::CancelRequested;
        control.changed.notify_all();
        self.event_tx
            .send(ThreadEvent {
                execution_id: execution_id.clone(),
                kind: ThreadEventKind::State(ExecutionTaskState::CancelRequested),
            })
            .map_err(|_| {
                EngineError::new(ErrorKind::Runtime, "execution event channel is unavailable")
            })?;
        Ok(())
    }

    pub fn snapshot(&self) -> ExecutionLaneSnapshot {
        let mut snapshot = self.scheduler.snapshot();
        let mut user_paused: Vec<_> = self.user_paused.values().cloned().collect();
        user_paused
            .sort_by(|left, right| left.execution_id.as_str().cmp(right.execution_id.as_str()));
        snapshot.user_paused = user_paused;
        snapshot
    }

    pub fn reorder_waiting(
        &mut self,
        expected_revision: u64,
        ordered_execution_ids: &[TaskId],
    ) -> Result<u64> {
        let revision = self
            .scheduler
            .reorder_waiting(expected_revision, ordered_execution_ids)?;
        self.sync_scheduled_states();
        Ok(revision)
    }

    pub fn drain_events(&mut self) -> Vec<ExecutionRuntimeEvent> {
        let mut events = Vec::new();
        while let Ok(event) = self.event_rx.try_recv() {
            if self
                .tasks
                .get(&event.execution_id)
                .is_some_and(|task| task.terminal_reported)
            {
                continue;
            }
            let (state, progress, output_path, error_code, message) = match event.kind {
                ThreadEventKind::State(state) => (state, None, None, None, None),
                ThreadEventKind::Started => (ExecutionTaskState::Running, None, None, None, None),
                ThreadEventKind::Progress(progress) => (
                    ExecutionTaskState::Running,
                    Some(progress),
                    None,
                    None,
                    None,
                ),
                ThreadEventKind::Paused(progress) => (
                    self.tasks
                        .get(&event.execution_id)
                        .map_or(ExecutionTaskState::Paused, |task| task.state),
                    Some(progress),
                    None,
                    None,
                    None,
                ),
                ThreadEventKind::Resumed => (ExecutionTaskState::Resuming, None, None, None, None),
                ThreadEventKind::Completed {
                    progress,
                    output_path,
                } => (
                    ExecutionTaskState::Completed,
                    Some(progress),
                    Some(output_path),
                    None,
                    None,
                ),
                ThreadEventKind::Cancelled(progress) => (
                    ExecutionTaskState::Cancelled,
                    Some(progress),
                    None,
                    None,
                    None,
                ),
                ThreadEventKind::Failed { code, message } => (
                    ExecutionTaskState::Failed,
                    None,
                    None,
                    Some(code),
                    Some(message),
                ),
            };
            if let Some(task) = self.tasks.get_mut(&event.execution_id) {
                task.state = state;
            }
            let terminal = matches!(
                state,
                ExecutionTaskState::Completed
                    | ExecutionTaskState::Failed
                    | ExecutionTaskState::Cancelled
            );
            if terminal && let Some(task) = self.tasks.get_mut(&event.execution_id) {
                task.terminal_reported = true;
            }
            if terminal
                && self
                    .scheduler
                    .snapshot()
                    .active_executions
                    .iter()
                    .any(|active| active.execution_id == event.execution_id)
            {
                self.scheduler
                    .clear_active_terminal(&event.execution_id, state)
                    .expect("terminal event belongs to the active execution");
                self.advance_available_slots();
                self.sync_scheduled_states();
            }
            let snapshot = self.scheduler.snapshot();
            let scheduled = snapshot
                .active_executions
                .iter()
                .chain(snapshot.video_resume_stack.iter())
                .chain(snapshot.auxiliary_resume_stack.iter())
                .find(|entry| entry.execution_id == event.execution_id);
            let resource_pool = self
                .tasks
                .get(&event.execution_id)
                .map(|task| task.plan.resource_pool)
                .expect("runtime event belongs to a known execution");
            events.push(ExecutionRuntimeEvent {
                execution_id: event.execution_id,
                resource_pool,
                sequence: self.next_event_sequence,
                state,
                pause_reason: scheduled.and_then(|entry| entry.pause_reason),
                preempted_by_execution_id: scheduled
                    .and_then(|entry| entry.preempted_by_execution_id.clone()),
                resume_depth: match resource_pool {
                    ExecutionResourcePool::Video => snapshot.video_resume_stack.len(),
                    ExecutionResourcePool::Auxiliary => snapshot.auxiliary_resume_stack.len(),
                },
                progress,
                output_path,
                error_code,
                message,
            });
            self.next_event_sequence = self.next_event_sequence.saturating_add(1);
        }
        events
    }

    fn start_next_available(&mut self) -> Result<Option<TaskId>> {
        let mut scheduler = std::mem::take(&mut self.scheduler);
        let mut control = RuntimeLaneControl {
            backend: Arc::clone(&self.services.backend),
            tasks: &mut self.tasks,
            event_tx: self.event_tx.clone(),
        };
        let result = scheduler.start_next_available(&mut control);
        self.scheduler = scheduler;
        result
    }

    fn advance_available_slots(&mut self) {
        loop {
            let failed_execution_id = self.scheduler.next_start_candidate();
            let error = match self.start_next_available() {
                Ok(Some(_)) => continue,
                Ok(None) => return,
                Err(error) => error,
            };
            let Some(failed_execution_id) = failed_execution_id else {
                return;
            };
            self.scheduler.remove_non_active(&failed_execution_id);
            if let Some(task) = self.tasks.get_mut(&failed_execution_id) {
                task.state = ExecutionTaskState::Failed;
                if let Some(control) = &task.control
                    && let Ok(mut state) = control.state.lock()
                {
                    state.cancel_requested = true;
                    control.changed.notify_all();
                }
            }
            let _ = self.event_tx.send(ThreadEvent {
                execution_id: failed_execution_id,
                kind: ThreadEventKind::Failed {
                    code: error.code(),
                    message: format!("execution could not start or resume: {}", error.message()),
                },
            });
        }
    }

    fn sync_scheduled_states(&mut self) {
        let snapshot = self.scheduler.snapshot();
        for entry in snapshot
            .active_executions
            .iter()
            .chain(snapshot.normal_waiting.iter())
            .chain(snapshot.video_resume_stack.iter())
            .chain(snapshot.auxiliary_resume_stack.iter())
        {
            if let Some(task) = self.tasks.get_mut(&entry.execution_id) {
                task.state = entry.state;
            }
        }
    }

    fn send_state_event(&self, execution_id: &TaskId, state: ExecutionTaskState) -> Result<()> {
        self.event_tx
            .send(ThreadEvent {
                execution_id: execution_id.clone(),
                kind: ThreadEventKind::State(state),
            })
            .map_err(|_| {
                EngineError::new(ErrorKind::Runtime, "execution event channel is unavailable")
            })
    }
}

struct RuntimeLaneControl<'a> {
    backend: Arc<dyn ExecutionBackend>,
    tasks: &'a mut HashMap<TaskId, RuntimeExecutionTask>,
    event_tx: Sender<ThreadEvent>,
}

impl ExecutionLaneControl for RuntimeLaneControl<'_> {
    fn pause_at_checkpoint(&mut self, execution_id: &TaskId) -> Result<ExecutionCheckpoint> {
        let task = self
            .tasks
            .get_mut(execution_id)
            .ok_or_else(|| EngineError::invalid_argument("active execution does not exist"))?;
        let control = task.control.as_ref().ok_or_else(|| {
            EngineError::invalid_task_state("active execution has no control channel")
        })?;
        let deadline = Instant::now() + SAFE_PAUSE_TIMEOUT;
        let mut state = control_state(control)?;
        state.pause_requested = true;
        task.state = ExecutionTaskState::PauseRequested;
        control.changed.notify_all();
        while !state.paused && !state.terminal {
            let now = Instant::now();
            if now >= deadline {
                state.pause_requested = false;
                return Err(EngineError::invalid_task_state(
                    "execution did not reach a safe pause checkpoint",
                ));
            }
            let wait = deadline.saturating_duration_since(now);
            let (next, timeout) = control.changed.wait_timeout(state, wait).map_err(|_| {
                EngineError::new(ErrorKind::Runtime, "execution control lock is poisoned")
            })?;
            state = next;
            if timeout.timed_out() && !state.paused {
                state.pause_requested = false;
                return Err(EngineError::invalid_task_state(
                    "execution did not reach a safe pause checkpoint",
                ));
            }
        }
        if state.terminal {
            return Err(EngineError::invalid_task_state(
                "execution reached a terminal state before it could be paused",
            ));
        }
        Ok(ExecutionCheckpoint {
            media_time_us: state.latest_progress.media_time_us,
            processed_bytes: state.latest_progress.processed_bytes,
            opaque_token: format!(
                "{}:{}:{}",
                execution_id,
                state.latest_progress.media_time_us,
                state.latest_progress.processed_bytes
            ),
        })
    }

    fn start(&mut self, execution_id: &TaskId) -> Result<()> {
        let task = self.tasks.get_mut(execution_id).ok_or_else(|| {
            EngineError::invalid_argument("execution start target does not exist")
        })?;
        if task.control.is_some() {
            return Err(EngineError::invalid_task_state(
                "execution has already been started",
            ));
        }
        let transaction = OutputTransaction::begin(&task.plan.source_path, &task.plan.output)?;
        let backend = Arc::clone(&self.backend);
        let request = ExecutionBackendRequest {
            source_path: task.plan.source_path.clone(),
            working_output_path: transaction.working_path().to_path_buf(),
            pipeline: task.plan.pipeline.clone(),
            configuration: task.plan.configuration.clone(),
        };
        let control = Arc::new(TaskControl::default());
        let thread_control = Arc::clone(&control);
        let event_tx = self.event_tx.clone();
        let thread_execution_id = execution_id.clone();
        thread::Builder::new()
            .name(format!("fll-execution-{execution_id}"))
            .spawn(move || {
                run_execution_thread(
                    thread_execution_id,
                    backend,
                    request,
                    transaction,
                    thread_control,
                    event_tx,
                );
            })
            .map_err(|error| {
                EngineError::with_source(
                    ErrorKind::Runtime,
                    "cannot start execution worker thread",
                    error,
                )
            })?;
        task.control = Some(control);
        task.state = ExecutionTaskState::Running;
        Ok(())
    }

    fn resume(&mut self, execution_id: &TaskId, _checkpoint: &ExecutionCheckpoint) -> Result<()> {
        let task = self.tasks.get_mut(execution_id).ok_or_else(|| {
            EngineError::invalid_argument("execution resume target does not exist")
        })?;
        let control = task.control.as_ref().ok_or_else(|| {
            EngineError::invalid_task_state("execution resume target has no control channel")
        })?;
        let mut state = control_state(control)?;
        if state.terminal {
            return Err(EngineError::invalid_task_state(
                "terminal execution cannot be resumed",
            ));
        }
        state.pause_requested = false;
        state.paused = false;
        task.state = ExecutionTaskState::Running;
        control.changed.notify_all();
        Ok(())
    }
}

fn run_execution_thread(
    execution_id: TaskId,
    backend: Arc<dyn ExecutionBackend>,
    request: ExecutionBackendRequest,
    transaction: OutputTransaction,
    control: Arc<TaskControl>,
    event_tx: Sender<ThreadEvent>,
) {
    let _ = event_tx.send(ThreadEvent {
        execution_id: execution_id.clone(),
        kind: ThreadEventKind::Started,
    });
    let mut observer = RuntimeBackendObserver {
        execution_id: execution_id.clone(),
        control: Arc::clone(&control),
        event_tx: event_tx.clone(),
        pause_event_sent: false,
        last_progress_event_at: None,
    };
    let outcome = backend.execute(&request, &mut observer);
    let event = match outcome {
        Ok(ExecutionBackendOutcome::Completed(progress)) => match transaction.commit() {
            Ok(output_path) => ThreadEventKind::Completed {
                progress,
                output_path,
            },
            Err(error) => ThreadEventKind::Failed {
                code: error.code(),
                message: error.message().to_owned(),
            },
        },
        Ok(ExecutionBackendOutcome::Cancelled(progress)) => {
            drop(transaction);
            ThreadEventKind::Cancelled(progress)
        }
        Err(error) => {
            drop(transaction);
            ThreadEventKind::Failed {
                code: error.code(),
                message: error.message().to_owned(),
            }
        }
    };
    if let Ok(mut state) = control.state.lock() {
        state.terminal = true;
        state.paused = false;
        control.changed.notify_all();
    }
    let _ = event_tx.send(ThreadEvent {
        execution_id,
        kind: event,
    });
}

struct RuntimeBackendObserver {
    execution_id: TaskId,
    control: Arc<TaskControl>,
    event_tx: Sender<ThreadEvent>,
    pause_event_sent: bool,
    last_progress_event_at: Option<Instant>,
}

impl ExecutionBackendObserver for RuntimeBackendObserver {
    fn on_progress(&mut self, progress: ExecutionProgress) -> ExecutionBackendControl {
        let now = Instant::now();
        if self
            .last_progress_event_at
            .is_none_or(|previous| now.duration_since(previous) >= PROGRESS_EVENT_INTERVAL)
        {
            let _ = self.event_tx.send(ThreadEvent {
                execution_id: self.execution_id.clone(),
                kind: ThreadEventKind::Progress(progress),
            });
            self.last_progress_event_at = Some(now);
        }
        let Ok(mut state) = self.control.state.lock() else {
            return ExecutionBackendControl::Cancel;
        };
        state.latest_progress = progress;
        if state.cancel_requested {
            return ExecutionBackendControl::Cancel;
        }
        if state.pause_requested {
            state.paused = true;
            if !self.pause_event_sent {
                let _ = self.event_tx.send(ThreadEvent {
                    execution_id: self.execution_id.clone(),
                    kind: ThreadEventKind::Paused(progress),
                });
                self.pause_event_sent = true;
            }
            self.control.changed.notify_all();
            while state.pause_requested && !state.cancel_requested {
                let Ok(next) = self.control.changed.wait(state) else {
                    return ExecutionBackendControl::Cancel;
                };
                state = next;
            }
            if state.cancel_requested {
                return ExecutionBackendControl::Cancel;
            }
            state.paused = false;
            self.pause_event_sent = false;
            let _ = self.event_tx.send(ThreadEvent {
                execution_id: self.execution_id.clone(),
                kind: ThreadEventKind::Resumed,
            });
            self.control.changed.notify_all();
        }
        ExecutionBackendControl::Continue
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn burst_progress_is_throttled_without_losing_control_progress() {
        let (event_tx, event_rx) = mpsc::channel();
        let control = Arc::new(TaskControl::default());
        let mut observer = RuntimeBackendObserver {
            execution_id: TaskId::new("execution-progress-throttle").unwrap(),
            control: Arc::clone(&control),
            event_tx,
            pause_event_sent: false,
            last_progress_event_at: None,
        };

        for value in 1..=100 {
            assert_eq!(
                observer.on_progress(ExecutionProgress {
                    media_time_us: value,
                    processed_bytes: value * 10,
                }),
                ExecutionBackendControl::Continue
            );
        }

        let progress_events = event_rx
            .try_iter()
            .filter(|event| matches!(event.kind, ThreadEventKind::Progress(_)))
            .count();
        assert_eq!(progress_events, 1);
        let state = control.state.lock().unwrap();
        assert_eq!(
            state.latest_progress,
            ExecutionProgress {
                media_time_us: 100,
                processed_bytes: 1_000,
            }
        );
    }
}

fn control_state(control: &TaskControl) -> Result<std::sync::MutexGuard<'_, TaskControlState>> {
    control
        .state
        .lock()
        .map_err(|_| EngineError::new(ErrorKind::Runtime, "execution control lock is poisoned"))
}
