use std::collections::{HashSet, VecDeque};
use std::path::PathBuf;

use framelean_core::{AnalysisId, EngineError, Result, TaskId};
use framelean_decision::RecalculateSelection;
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

use crate::{AnalysisRevision, RequestContext};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum OutputCollisionPolicy {
    FailIfExists,
    GenerateUnique,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct ExecutionOutputRequest {
    pub requested_path: PathBuf,
    pub collision_policy: OutputCollisionPolicy,
}

impl ExecutionOutputRequest {
    pub(crate) fn validate(&self) -> framelean_core::Result<()> {
        if self.requested_path.as_os_str().is_empty() || self.requested_path.file_name().is_none() {
            return Err(framelean_core::EngineError::invalid_argument(
                "execution output path must identify a file",
            ));
        }
        if !self.requested_path.is_absolute() {
            return Err(framelean_core::EngineError::invalid_argument(
                "execution output path must be absolute",
            ));
        }
        Ok(())
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct ExecutionSubmissionRequest {
    pub analysis_id: AnalysisId,
    pub expected_revision: AnalysisRevision,
    pub selection: RecalculateSelection,
    pub output: ExecutionOutputRequest,
    #[serde(default)]
    pub context: RequestContext,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum ExecutionTaskState {
    Queued,
    Running,
    Preempting,
    Preempted,
    Resuming,
    PauseRequested,
    Paused,
    CancelRequested,
    Cancelled,
    Completed,
    Failed,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct ExecutionSubmissionResult {
    pub execution_id: TaskId,
    pub state: ExecutionTaskState,
    pub queue_position: usize,
    pub queue_revision: u64,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum ExecutionPauseReason {
    User,
    Preemption,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum ExecutionResourcePool {
    Video,
    Auxiliary,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct ExecutionCheckpoint {
    pub media_time_us: u64,
    pub processed_bytes: u64,
    pub opaque_token: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct ScheduledExecution {
    pub execution_id: TaskId,
    pub resource_pool: ExecutionResourcePool,
    pub state: ExecutionTaskState,
    pub pause_reason: Option<ExecutionPauseReason>,
    pub preempted_by_execution_id: Option<TaskId>,
    pub checkpoint: Option<ExecutionCheckpoint>,
}

impl ScheduledExecution {
    pub fn queued(execution_id: TaskId, resource_pool: ExecutionResourcePool) -> Self {
        Self {
            execution_id,
            resource_pool,
            state: ExecutionTaskState::Queued,
            pause_reason: None,
            preempted_by_execution_id: None,
            checkpoint: None,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct ExecutionLaneSnapshot {
    pub queue_revision: u64,
    pub active_executions: Vec<ScheduledExecution>,
    pub normal_waiting: Vec<ScheduledExecution>,
    pub video_resume_stack: Vec<ScheduledExecution>,
    pub auxiliary_resume_stack: Vec<ScheduledExecution>,
    pub user_paused: Vec<ScheduledExecution>,
}

pub trait ExecutionLaneControl {
    fn pause_at_checkpoint(&mut self, execution_id: &TaskId) -> Result<ExecutionCheckpoint>;
    fn start(&mut self, execution_id: &TaskId) -> Result<()>;
    fn resume(&mut self, execution_id: &TaskId, checkpoint: &ExecutionCheckpoint) -> Result<()>;
}

#[derive(Debug, Default)]
pub struct ExecutionScheduler {
    queue_revision: u64,
    active_executions: Vec<ScheduledExecution>,
    normal_waiting: VecDeque<ScheduledExecution>,
    video_resume_stack: Vec<ScheduledExecution>,
    auxiliary_resume_stack: Vec<ScheduledExecution>,
}

impl ExecutionScheduler {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn enqueue(
        &mut self,
        execution_id: TaskId,
        resource_pool: ExecutionResourcePool,
    ) -> Result<usize> {
        if self.contains(&execution_id) {
            return Err(EngineError::invalid_argument(
                "execution is already present in the scheduler",
            ));
        }
        self.normal_waiting
            .push_back(ScheduledExecution::queued(execution_id, resource_pool));
        self.bump_revision();
        Ok(self.normal_waiting.len())
    }

    pub fn snapshot(&self) -> ExecutionLaneSnapshot {
        ExecutionLaneSnapshot {
            queue_revision: self.queue_revision,
            active_executions: self.active_executions.clone(),
            normal_waiting: self.normal_waiting.iter().cloned().collect(),
            video_resume_stack: self.video_resume_stack.clone(),
            auxiliary_resume_stack: self.auxiliary_resume_stack.clone(),
            user_paused: Vec::new(),
        }
    }

    pub fn start_available(
        &mut self,
        control: &mut dyn ExecutionLaneControl,
    ) -> Result<Vec<TaskId>> {
        let mut started = Vec::new();
        loop {
            let Some(execution_id) = self.start_next_available(control)? else {
                return Ok(started);
            };
            started.push(execution_id);
        }
    }

    pub fn start_next_available(
        &mut self,
        control: &mut dyn ExecutionLaneControl,
    ) -> Result<Option<TaskId>> {
        for pool in [
            ExecutionResourcePool::Video,
            ExecutionResourcePool::Auxiliary,
        ] {
            if let Some(execution_id) = self.start_one(pool, control)? {
                return Ok(Some(execution_id));
            }
        }
        Ok(None)
    }

    pub fn preempt_and_start(
        &mut self,
        target_execution_id: &TaskId,
        control: &mut dyn ExecutionLaneControl,
    ) -> Result<()> {
        if self
            .active_executions
            .iter()
            .any(|entry| entry.execution_id == *target_execution_id)
        {
            return Ok(());
        }
        let target_position = self
            .normal_waiting
            .iter()
            .position(|entry| entry.execution_id == *target_execution_id)
            .ok_or_else(|| {
                EngineError::invalid_task_state(
                    "preemption target is not in the normal waiting queue",
                )
            })?;

        let target_pool = self.normal_waiting[target_position].resource_pool;
        let mut paused = Vec::new();
        for victim_id in self.preemption_victims(target_pool) {
            match control.pause_at_checkpoint(&victim_id) {
                Ok(checkpoint) => paused.push((victim_id, checkpoint)),
                Err(error) => {
                    for (paused_id, checkpoint) in paused.iter().rev() {
                        let _ = control.resume(paused_id, checkpoint);
                    }
                    return Err(error);
                }
            }
        }
        if let Err(start_error) = control.start(target_execution_id) {
            let mut restore_error = None;
            for (paused_id, checkpoint) in paused.iter().rev() {
                if let Err(error) = control.resume(paused_id, checkpoint) {
                    restore_error = Some(error);
                    break;
                }
            }
            return match restore_error {
                None => Err(start_error),
                Some(resume_error) => Err(EngineError::invalid_task_state(format!(
                    "preemption target failed to start and an active execution could not be restored: {}; restore error: {}",
                    start_error.message(),
                    resume_error.message()
                ))),
            };
        }

        for (victim_id, checkpoint) in paused {
            let position = self
                .active_executions
                .iter()
                .position(|entry| entry.execution_id == victim_id)
                .expect("paused execution must still be active");
            let mut preempted = self.active_executions.remove(position);
            preempted.state = ExecutionTaskState::Preempted;
            preempted.pause_reason = Some(ExecutionPauseReason::Preemption);
            preempted.preempted_by_execution_id = Some(target_execution_id.clone());
            preempted.checkpoint = Some(checkpoint);
            self.resume_stack_mut(preempted.resource_pool)
                .push(preempted);
        }

        let mut target = self
            .normal_waiting
            .remove(target_position)
            .expect("preemption target position was validated before removal");
        target.state = ExecutionTaskState::Running;
        target.pause_reason = None;
        target.preempted_by_execution_id = None;
        target.checkpoint = None;
        self.active_executions.push(target);
        self.bump_revision();
        Ok(())
    }

    pub fn finish_active(
        &mut self,
        execution_id: &TaskId,
        terminal_state: ExecutionTaskState,
        control: &mut dyn ExecutionLaneControl,
    ) -> Result<Vec<TaskId>> {
        self.clear_active_terminal(execution_id, terminal_state)?;
        self.start_available(control)
    }

    pub fn clear_active_terminal(
        &mut self,
        execution_id: &TaskId,
        terminal_state: ExecutionTaskState,
    ) -> Result<TaskId> {
        if !matches!(
            terminal_state,
            ExecutionTaskState::Completed
                | ExecutionTaskState::Failed
                | ExecutionTaskState::Cancelled
        ) {
            return Err(EngineError::invalid_argument(
                "finish_active requires a terminal execution state",
            ));
        }
        let position = self
            .active_executions
            .iter()
            .position(|entry| entry.execution_id == *execution_id)
            .ok_or_else(|| EngineError::invalid_task_state("execution is not active"))?;
        let finished = self.active_executions.remove(position);
        self.bump_revision();
        Ok(finished.execution_id)
    }

    pub fn pause_active_for_user(
        &mut self,
        execution_id: &TaskId,
        control: &mut dyn ExecutionLaneControl,
    ) -> Result<ScheduledExecution> {
        let position = self
            .active_executions
            .iter()
            .position(|entry| entry.execution_id == *execution_id)
            .ok_or_else(|| EngineError::invalid_task_state("execution is not active"))?;
        let checkpoint = control.pause_at_checkpoint(execution_id)?;
        let mut paused = self.active_executions.remove(position);
        paused.state = ExecutionTaskState::Paused;
        paused.pause_reason = Some(ExecutionPauseReason::User);
        paused.checkpoint = Some(checkpoint.clone());
        self.bump_revision();
        Ok(paused)
    }

    pub fn resume_user_paused(
        &mut self,
        mut paused: ScheduledExecution,
        control: &mut dyn ExecutionLaneControl,
    ) -> Result<()> {
        if !self.has_capacity(paused.resource_pool) {
            return Err(EngineError::invalid_task_state(
                "cannot resume a user-paused execution while its resource pool is full",
            ));
        }
        if paused.pause_reason != Some(ExecutionPauseReason::User)
            || paused.state != ExecutionTaskState::Paused
        {
            return Err(EngineError::invalid_task_state(
                "only a user-paused execution can be resumed explicitly",
            ));
        }
        let checkpoint = paused.checkpoint.as_ref().ok_or_else(|| {
            EngineError::invalid_task_state("user-paused execution has no checkpoint")
        })?;
        control.resume(&paused.execution_id, checkpoint)?;
        paused.state = ExecutionTaskState::Running;
        paused.pause_reason = None;
        paused.checkpoint = None;
        self.active_executions.push(paused);
        self.bump_revision();
        Ok(())
    }

    pub fn reorder_waiting(
        &mut self,
        expected_revision: u64,
        ordered_execution_ids: &[TaskId],
    ) -> Result<u64> {
        if expected_revision != self.queue_revision {
            return Err(EngineError::with_code(
                framelean_core::ErrorKind::Runtime,
                framelean_core::EngineErrorCode::QueueRevisionConflict,
                format!(
                    "execution queue revision conflict: expected {expected_revision}, current {}",
                    self.queue_revision
                ),
            ));
        }
        let mut requested = HashSet::with_capacity(ordered_execution_ids.len());
        for execution_id in ordered_execution_ids {
            if !requested.insert(execution_id.as_str()) {
                return Err(EngineError::invalid_argument(
                    "execution queue order contains a duplicate execution id",
                ));
            }
            if !self
                .normal_waiting
                .iter()
                .any(|entry| entry.execution_id == *execution_id)
            {
                return Err(EngineError::invalid_argument(
                    "execution queue order contains an unknown execution id",
                ));
            }
        }
        if self
            .normal_waiting
            .iter()
            .any(|entry| !requested.contains(entry.execution_id.as_str()))
        {
            return Err(EngineError::invalid_argument(
                "execution queue order omits a waiting execution id",
            ));
        }
        let changed = self
            .normal_waiting
            .iter()
            .map(|entry| &entry.execution_id)
            .ne(ordered_execution_ids.iter());
        if !changed {
            return Ok(self.queue_revision);
        }
        let mut remaining = std::mem::take(&mut self.normal_waiting);
        for execution_id in ordered_execution_ids {
            let index = remaining
                .iter()
                .position(|entry| entry.execution_id == *execution_id)
                .expect("waiting execution order was validated before mutation");
            self.normal_waiting.push_back(
                remaining
                    .remove(index)
                    .expect("validated waiting execution must still exist"),
            );
        }
        self.bump_revision();
        Ok(self.queue_revision)
    }

    pub fn remove_non_active(&mut self, execution_id: &TaskId) -> Option<ScheduledExecution> {
        if let Some(position) = self
            .normal_waiting
            .iter()
            .position(|entry| entry.execution_id == *execution_id)
        {
            let removed = self.normal_waiting.remove(position);
            if removed.is_some() {
                self.bump_revision();
            }
            return removed;
        }
        for pool in [
            ExecutionResourcePool::Video,
            ExecutionResourcePool::Auxiliary,
        ] {
            let stack = self.resume_stack_mut(pool);
            if let Some(position) = stack
                .iter()
                .position(|entry| entry.execution_id == *execution_id)
            {
                let removed = stack.remove(position);
                self.bump_revision();
                return Some(removed);
            }
        }
        None
    }

    pub fn resource_pool(&self, execution_id: &TaskId) -> Option<ExecutionResourcePool> {
        self.active_executions
            .iter()
            .chain(self.normal_waiting.iter())
            .chain(self.video_resume_stack.iter())
            .chain(self.auxiliary_resume_stack.iter())
            .find(|entry| entry.execution_id == *execution_id)
            .map(|entry| entry.resource_pool)
    }

    pub fn resume_depth(&self, pool: ExecutionResourcePool) -> usize {
        self.resume_stack(pool).len()
    }

    pub fn preemption_victims_for(&self, execution_id: &TaskId) -> Result<Vec<TaskId>> {
        if self
            .active_executions
            .iter()
            .any(|entry| entry.execution_id == *execution_id)
        {
            return Ok(Vec::new());
        }
        let pool = self
            .normal_waiting
            .iter()
            .find(|entry| entry.execution_id == *execution_id)
            .map(|entry| entry.resource_pool)
            .ok_or_else(|| {
                EngineError::invalid_task_state(
                    "preemption target is not in the normal waiting queue",
                )
            })?;
        Ok(self.preemption_victims(pool))
    }

    pub fn next_start_candidate(&self) -> Option<TaskId> {
        for pool in [
            ExecutionResourcePool::Video,
            ExecutionResourcePool::Auxiliary,
        ] {
            if !self.has_capacity(pool) {
                continue;
            }
            if let Some(entry) = self.resume_stack(pool).last() {
                return Some(entry.execution_id.clone());
            }
            if let Some(entry) = self
                .normal_waiting
                .iter()
                .find(|entry| entry.resource_pool == pool)
            {
                return Some(entry.execution_id.clone());
            }
        }
        None
    }

    fn start_one(
        &mut self,
        pool: ExecutionResourcePool,
        control: &mut dyn ExecutionLaneControl,
    ) -> Result<Option<TaskId>> {
        if !self.has_capacity(pool) {
            return Ok(None);
        }
        if let Some(entry) = self.resume_stack(pool).last() {
            let checkpoint = entry.checkpoint.as_ref().ok_or_else(|| {
                EngineError::invalid_task_state("preempted execution has no checkpoint")
            })?;
            control.resume(&entry.execution_id, checkpoint)?;
            let mut resumed = self
                .resume_stack_mut(pool)
                .pop()
                .expect("resume stack entry was inspected before removal");
            resumed.state = ExecutionTaskState::Running;
            resumed.pause_reason = None;
            resumed.preempted_by_execution_id = None;
            resumed.checkpoint = None;
            let execution_id = resumed.execution_id.clone();
            self.active_executions.push(resumed);
            self.bump_revision();
            return Ok(Some(execution_id));
        }
        let Some(position) = self
            .normal_waiting
            .iter()
            .position(|entry| entry.resource_pool == pool)
        else {
            return Ok(None);
        };
        let execution_id = self.normal_waiting[position].execution_id.clone();
        control.start(&execution_id)?;
        let mut started = self
            .normal_waiting
            .remove(position)
            .expect("waiting execution position was validated before removal");
        started.state = ExecutionTaskState::Running;
        self.active_executions.push(started);
        self.bump_revision();
        Ok(Some(execution_id))
    }

    fn preemption_victims(&self, target_pool: ExecutionResourcePool) -> Vec<TaskId> {
        let mut victims = Vec::new();
        match target_pool {
            ExecutionResourcePool::Video => {
                if let Some(active_video) = self
                    .active_executions
                    .iter()
                    .rev()
                    .find(|entry| entry.resource_pool == ExecutionResourcePool::Video)
                {
                    victims.push(active_video.execution_id.clone());
                }
                if self.active_count(ExecutionResourcePool::Auxiliary) > 1
                    && let Some(active_auxiliary) = self
                        .active_executions
                        .iter()
                        .rev()
                        .find(|entry| entry.resource_pool == ExecutionResourcePool::Auxiliary)
                {
                    victims.push(active_auxiliary.execution_id.clone());
                }
            }
            ExecutionResourcePool::Auxiliary => {
                if !self.has_capacity(ExecutionResourcePool::Auxiliary)
                    && let Some(active_auxiliary) = self
                        .active_executions
                        .iter()
                        .rev()
                        .find(|entry| entry.resource_pool == ExecutionResourcePool::Auxiliary)
                {
                    victims.push(active_auxiliary.execution_id.clone());
                }
            }
        }
        victims
    }

    fn has_capacity(&self, pool: ExecutionResourcePool) -> bool {
        match pool {
            ExecutionResourcePool::Video => {
                self.active_count(ExecutionResourcePool::Video) == 0
                    && self.active_count(ExecutionResourcePool::Auxiliary) <= 1
            }
            ExecutionResourcePool::Auxiliary => {
                let capacity = if self.active_count(ExecutionResourcePool::Video) == 0 {
                    2
                } else {
                    1
                };
                self.active_count(ExecutionResourcePool::Auxiliary) < capacity
            }
        }
    }

    fn active_count(&self, pool: ExecutionResourcePool) -> usize {
        self.active_executions
            .iter()
            .filter(|entry| entry.resource_pool == pool)
            .count()
    }

    fn resume_stack(&self, pool: ExecutionResourcePool) -> &Vec<ScheduledExecution> {
        match pool {
            ExecutionResourcePool::Video => &self.video_resume_stack,
            ExecutionResourcePool::Auxiliary => &self.auxiliary_resume_stack,
        }
    }

    fn resume_stack_mut(&mut self, pool: ExecutionResourcePool) -> &mut Vec<ScheduledExecution> {
        match pool {
            ExecutionResourcePool::Video => &mut self.video_resume_stack,
            ExecutionResourcePool::Auxiliary => &mut self.auxiliary_resume_stack,
        }
    }

    fn bump_revision(&mut self) {
        self.queue_revision = self.queue_revision.saturating_add(1);
    }

    fn contains(&self, execution_id: &TaskId) -> bool {
        self.active_executions
            .iter()
            .any(|entry| entry.execution_id == *execution_id)
            || self
                .normal_waiting
                .iter()
                .any(|entry| entry.execution_id == *execution_id)
            || self
                .video_resume_stack
                .iter()
                .any(|entry| entry.execution_id == *execution_id)
            || self
                .auxiliary_resume_stack
                .iter()
                .any(|entry| entry.execution_id == *execution_id)
    }
}

#[cfg(test)]
mod tests {
    use std::path::PathBuf;

    use super::*;

    #[derive(Default)]
    struct RecordingControl {
        calls: Vec<String>,
        fail_pause_for: Option<TaskId>,
        fail_start_for: Option<TaskId>,
    }

    impl ExecutionLaneControl for RecordingControl {
        fn pause_at_checkpoint(&mut self, execution_id: &TaskId) -> Result<ExecutionCheckpoint> {
            self.calls.push(format!("pause:{execution_id}"));
            if self.fail_pause_for.as_ref() == Some(execution_id) {
                return Err(EngineError::invalid_task_state("checkpoint pause failed"));
            }
            Ok(ExecutionCheckpoint {
                media_time_us: self.calls.len() as u64,
                processed_bytes: self.calls.len() as u64 * 100,
                opaque_token: format!("checkpoint:{execution_id}"),
            })
        }

        fn start(&mut self, execution_id: &TaskId) -> Result<()> {
            self.calls.push(format!("start:{execution_id}"));
            if self.fail_start_for.as_ref() == Some(execution_id) {
                return Err(EngineError::invalid_task_state("execution start failed"));
            }
            Ok(())
        }

        fn resume(
            &mut self,
            execution_id: &TaskId,
            _checkpoint: &ExecutionCheckpoint,
        ) -> Result<()> {
            self.calls.push(format!("resume:{execution_id}"));
            Ok(())
        }
    }

    fn execution_id(value: &str) -> TaskId {
        TaskId::new(value).unwrap()
    }

    #[test]
    fn output_request_requires_an_absolute_file_path() {
        let relative = ExecutionOutputRequest {
            requested_path: PathBuf::from("output.mp4"),
            collision_policy: OutputCollisionPolicy::FailIfExists,
        };
        let directory = ExecutionOutputRequest {
            requested_path: PathBuf::from("/"),
            collision_policy: OutputCollisionPolicy::FailIfExists,
        };

        assert!(relative.validate().is_err());
        assert!(directory.validate().is_err());
    }

    #[test]
    fn video_pool_never_runs_two_tasks_at_once() {
        let v1 = execution_id("v1");
        let v2 = execution_id("v2");
        let mut scheduler = ExecutionScheduler::new();
        let mut control = RecordingControl::default();
        scheduler
            .enqueue(v1.clone(), ExecutionResourcePool::Video)
            .unwrap();
        scheduler
            .enqueue(v2.clone(), ExecutionResourcePool::Video)
            .unwrap();

        assert_eq!(
            scheduler.start_available(&mut control).unwrap(),
            vec![v1.clone()]
        );
        let snapshot = scheduler.snapshot();
        assert_eq!(snapshot.active_executions.len(), 1);
        assert_eq!(snapshot.active_executions[0].execution_id, v1);
        assert_eq!(snapshot.normal_waiting[0].execution_id, v2);
    }

    #[test]
    fn video_and_one_auxiliary_task_can_run_together() {
        let video = execution_id("video");
        let image = execution_id("image");
        let mut scheduler = ExecutionScheduler::new();
        let mut control = RecordingControl::default();
        scheduler
            .enqueue(video.clone(), ExecutionResourcePool::Video)
            .unwrap();
        scheduler
            .enqueue(image.clone(), ExecutionResourcePool::Auxiliary)
            .unwrap();

        assert_eq!(
            scheduler.start_available(&mut control).unwrap(),
            vec![video.clone(), image.clone()]
        );
        let snapshot = scheduler.snapshot();
        assert_eq!(snapshot.active_executions.len(), 2);
        assert!(snapshot.normal_waiting.is_empty());
    }

    #[test]
    fn later_slot_start_failure_keeps_the_successful_slot_active() {
        let video = execution_id("video");
        let image = execution_id("image");
        let mut scheduler = ExecutionScheduler::new();
        let mut control = RecordingControl {
            fail_start_for: Some(image.clone()),
            ..RecordingControl::default()
        };
        scheduler
            .enqueue(video.clone(), ExecutionResourcePool::Video)
            .unwrap();
        scheduler
            .enqueue(image.clone(), ExecutionResourcePool::Auxiliary)
            .unwrap();

        assert!(scheduler.start_available(&mut control).is_err());

        let snapshot = scheduler.snapshot();
        assert_eq!(snapshot.active_executions[0].execution_id, video);
        assert_eq!(snapshot.normal_waiting[0].execution_id, image.clone());
        assert_eq!(scheduler.next_start_candidate(), Some(image));
    }

    #[test]
    fn two_auxiliary_tasks_run_when_video_pool_is_idle() {
        let image = execution_id("image");
        let audio = execution_id("audio");
        let waiting = execution_id("waiting");
        let mut scheduler = ExecutionScheduler::new();
        let mut control = RecordingControl::default();
        for execution_id in [&image, &audio, &waiting] {
            scheduler
                .enqueue(execution_id.clone(), ExecutionResourcePool::Auxiliary)
                .unwrap();
        }

        assert_eq!(
            scheduler.start_available(&mut control).unwrap(),
            vec![image.clone(), audio.clone()]
        );
        let snapshot = scheduler.snapshot();
        assert_eq!(snapshot.active_executions.len(), 2);
        assert_eq!(snapshot.normal_waiting[0].execution_id, waiting);
    }

    #[test]
    fn starting_video_safely_shrinks_auxiliary_capacity() {
        let image = execution_id("image");
        let audio = execution_id("audio");
        let video = execution_id("video");
        let mut scheduler = ExecutionScheduler::new();
        let mut control = RecordingControl::default();
        scheduler
            .enqueue(image.clone(), ExecutionResourcePool::Auxiliary)
            .unwrap();
        scheduler
            .enqueue(audio.clone(), ExecutionResourcePool::Auxiliary)
            .unwrap();
        scheduler.start_available(&mut control).unwrap();
        scheduler
            .enqueue(video.clone(), ExecutionResourcePool::Video)
            .unwrap();

        scheduler.preempt_and_start(&video, &mut control).unwrap();

        let snapshot = scheduler.snapshot();
        assert_eq!(snapshot.active_executions.len(), 2);
        assert!(
            snapshot
                .active_executions
                .iter()
                .any(|entry| entry.execution_id == video)
        );
        assert_eq!(snapshot.auxiliary_resume_stack[0].execution_id, audio);
        assert_eq!(control.calls.last().unwrap(), "start:video");
    }

    #[test]
    fn failed_video_capacity_shrink_keeps_auxiliary_tasks_running() {
        let image = execution_id("image");
        let audio = execution_id("audio");
        let video = execution_id("video");
        let mut scheduler = ExecutionScheduler::new();
        let mut control = RecordingControl::default();
        scheduler
            .enqueue(image.clone(), ExecutionResourcePool::Auxiliary)
            .unwrap();
        scheduler
            .enqueue(audio.clone(), ExecutionResourcePool::Auxiliary)
            .unwrap();
        scheduler.start_available(&mut control).unwrap();
        scheduler
            .enqueue(video.clone(), ExecutionResourcePool::Video)
            .unwrap();
        control.fail_pause_for = Some(audio.clone());

        assert!(scheduler.preempt_and_start(&video, &mut control).is_err());

        let snapshot = scheduler.snapshot();
        assert_eq!(snapshot.active_executions.len(), 2);
        assert_eq!(snapshot.normal_waiting[0].execution_id, video);
        assert!(snapshot.auxiliary_resume_stack.is_empty());
    }

    #[test]
    fn nested_preemption_resumes_in_per_pool_lifo_order() {
        let a1 = execution_id("a1");
        let a2 = execution_id("a2");
        let a3 = execution_id("a3");
        let mut scheduler = ExecutionScheduler::new();
        let mut control = RecordingControl::default();
        for execution_id in [&a1, &a2, &a3] {
            scheduler
                .enqueue(execution_id.clone(), ExecutionResourcePool::Video)
                .unwrap();
        }

        scheduler.start_available(&mut control).unwrap();
        scheduler.preempt_and_start(&a3, &mut control).unwrap();
        scheduler.preempt_and_start(&a2, &mut control).unwrap();

        let snapshot = scheduler.snapshot();
        assert_eq!(snapshot.active_executions[0].execution_id, a2);
        assert_eq!(
            snapshot
                .video_resume_stack
                .iter()
                .map(|entry| entry.execution_id.clone())
                .collect::<Vec<_>>(),
            [a1.clone(), a3.clone()]
        );
        assert_eq!(
            scheduler
                .finish_active(&a2, ExecutionTaskState::Completed, &mut control)
                .unwrap(),
            vec![a3.clone()]
        );
        assert_eq!(
            scheduler
                .finish_active(&a3, ExecutionTaskState::Failed, &mut control)
                .unwrap(),
            vec![a1.clone()]
        );
        assert_eq!(
            control.calls,
            [
                "start:a1",
                "pause:a1",
                "start:a3",
                "pause:a3",
                "start:a2",
                "resume:a3",
                "resume:a1",
            ]
        );
    }

    #[test]
    fn failed_safe_pause_keeps_active_and_target_in_place() {
        let active = execution_id("active");
        let target = execution_id("target");
        let mut scheduler = ExecutionScheduler::new();
        let mut control = RecordingControl::default();
        scheduler
            .enqueue(active.clone(), ExecutionResourcePool::Video)
            .unwrap();
        scheduler
            .enqueue(target.clone(), ExecutionResourcePool::Video)
            .unwrap();
        scheduler.start_available(&mut control).unwrap();
        control.fail_pause_for = Some(active.clone());

        assert!(scheduler.preempt_and_start(&target, &mut control).is_err());
        let snapshot = scheduler.snapshot();
        assert_eq!(snapshot.active_executions[0].execution_id, active);
        assert_eq!(snapshot.normal_waiting[0].execution_id, target);
        assert!(snapshot.video_resume_stack.is_empty());
    }

    #[test]
    fn failed_target_start_restores_active_without_mutating_scheduler() {
        let active = execution_id("active");
        let target = execution_id("target");
        let mut scheduler = ExecutionScheduler::new();
        let mut control = RecordingControl::default();
        scheduler
            .enqueue(active.clone(), ExecutionResourcePool::Video)
            .unwrap();
        scheduler
            .enqueue(target.clone(), ExecutionResourcePool::Video)
            .unwrap();
        scheduler.start_available(&mut control).unwrap();
        control.fail_start_for = Some(target.clone());

        assert!(scheduler.preempt_and_start(&target, &mut control).is_err());
        let snapshot = scheduler.snapshot();
        assert_eq!(snapshot.active_executions[0].execution_id, active);
        assert_eq!(snapshot.normal_waiting[0].execution_id, target);
        assert!(snapshot.video_resume_stack.is_empty());
        assert_eq!(
            control.calls,
            [
                "start:active",
                "pause:active",
                "start:target",
                "resume:active"
            ]
        );
    }

    #[test]
    fn user_paused_execution_is_not_added_to_the_auto_resume_stack() {
        let active = execution_id("active");
        let waiting = execution_id("waiting");
        let mut scheduler = ExecutionScheduler::new();
        let mut control = RecordingControl::default();
        scheduler
            .enqueue(active.clone(), ExecutionResourcePool::Video)
            .unwrap();
        scheduler
            .enqueue(waiting.clone(), ExecutionResourcePool::Video)
            .unwrap();
        scheduler.start_available(&mut control).unwrap();

        let paused = scheduler
            .pause_active_for_user(&active, &mut control)
            .unwrap();
        let snapshot = scheduler.snapshot();
        assert!(snapshot.active_executions.is_empty());
        assert!(snapshot.video_resume_stack.is_empty());
        assert_eq!(snapshot.normal_waiting[0].execution_id, waiting.clone());
        assert_eq!(
            scheduler.start_available(&mut control).unwrap(),
            vec![waiting]
        );
        assert_eq!(paused.execution_id, active);
        assert_eq!(paused.pause_reason, Some(ExecutionPauseReason::User));
    }
}
