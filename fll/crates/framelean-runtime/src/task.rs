use framelean_core::{EngineError, Result, TaskId};
use framelean_media::processor::{ProcessInput, ProcessOutput};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PipelineSpec {
    processors: Vec<framelean_core::ProcessorId>,
}

impl PipelineSpec {
    pub fn new(processors: Vec<framelean_core::ProcessorId>) -> Self {
        Self { processors }
    }

    pub fn processors(&self) -> &[framelean_core::ProcessorId] {
        &self.processors
    }
}

#[derive(Debug, PartialEq, Eq)]
pub struct TaskRequest {
    pipeline: PipelineSpec,
    input: ProcessInput,
}

impl TaskRequest {
    pub fn new(pipeline: PipelineSpec, input: ProcessInput) -> Self {
        Self { pipeline, input }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TaskState {
    Created,
    Queued,
    Running,
    Completed,
    Failed,
}

#[derive(Debug)]
pub struct Task {
    id: TaskId,
    state: TaskState,
    pipeline: PipelineSpec,
    pending_input: Option<ProcessInput>,
    output: Option<ProcessOutput>,
    failure: Option<EngineError>,
}

impl Task {
    pub fn from_request(id: TaskId, request: TaskRequest) -> Self {
        Self {
            id,
            state: TaskState::Created,
            pipeline: request.pipeline,
            pending_input: Some(request.input),
            output: None,
            failure: None,
        }
    }

    pub fn id(&self) -> &TaskId {
        &self.id
    }

    pub fn state(&self) -> TaskState {
        self.state
    }

    pub fn pipeline(&self) -> &PipelineSpec {
        &self.pipeline
    }

    pub fn output(&self) -> Option<&ProcessOutput> {
        self.output.as_ref()
    }

    pub fn failure(&self) -> Option<&EngineError> {
        self.failure.as_ref()
    }

    pub(crate) fn queue(&mut self) -> Result<()> {
        self.transition(TaskState::Created, TaskState::Queued)
    }

    pub(crate) fn start(&mut self) -> Result<()> {
        self.transition(TaskState::Queued, TaskState::Running)
    }

    pub(crate) fn take_input(&mut self) -> Result<ProcessInput> {
        self.pending_input
            .take()
            .ok_or_else(|| EngineError::invalid_argument("task input has already been consumed"))
    }

    pub(crate) fn complete(&mut self, output: ProcessOutput) -> Result<()> {
        self.transition(TaskState::Running, TaskState::Completed)?;
        self.output = Some(output);
        self.failure = None;
        Ok(())
    }

    pub(crate) fn fail(&mut self, error: EngineError) -> Result<()> {
        self.transition(TaskState::Running, TaskState::Failed)?;
        self.output = None;
        self.failure = Some(error);
        Ok(())
    }

    fn transition(&mut self, expected: TaskState, next: TaskState) -> Result<()> {
        if self.state != expected {
            return Err(EngineError::invalid_task_state(format!(
                "cannot transition task {} from {:?} to {:?}",
                self.id, self.state, next
            )));
        }
        self.state = next;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use framelean_core::{ErrorKind, ProcessorId};
    use framelean_media::{MediaBuffer, StreamId, VideoFrame};

    use super::*;

    fn task() -> Task {
        Task::from_request(
            TaskId::new("task-test").unwrap(),
            TaskRequest::new(
                PipelineSpec::new(vec![ProcessorId::new("processor-test").unwrap()]),
                ProcessInput::Video(VideoFrame::new(
                    StreamId::new(0),
                    None,
                    MediaBuffer::new(vec![1]),
                )),
            ),
        )
    }

    fn output() -> ProcessOutput {
        ProcessOutput::Video(VideoFrame::new(
            StreamId::new(0),
            None,
            MediaBuffer::new(vec![1]),
        ))
    }

    fn running_task() -> Task {
        let mut task = task();
        task.queue().unwrap();
        task.start().unwrap();
        task
    }

    #[test]
    fn task_enforces_lifecycle_transitions() {
        let mut task = task();
        assert_eq!(task.state(), TaskState::Created);
        task.queue().unwrap();
        assert_eq!(task.state(), TaskState::Queued);
        task.start().unwrap();
        assert_eq!(task.state(), TaskState::Running);
        task.complete(output()).unwrap();
        assert_eq!(task.state(), TaskState::Completed);
        assert!(task.output().is_some());
        assert!(task.failure().is_none());
    }

    #[test]
    fn invalid_transition_returns_typed_error() {
        let mut task = task();
        let error = task.start().unwrap_err();
        assert_eq!(error.kind(), ErrorKind::InvalidTaskState);
    }

    #[test]
    fn created_task_cannot_complete_or_fail() {
        let mut completing = task();
        let mut failing = task();

        assert_eq!(
            completing.complete(output()).unwrap_err().kind(),
            ErrorKind::InvalidTaskState
        );
        assert_eq!(
            failing
                .fail(EngineError::new(ErrorKind::Runtime, "test"))
                .unwrap_err()
                .kind(),
            ErrorKind::InvalidTaskState
        );
    }

    #[test]
    fn running_task_cannot_return_to_queue() {
        let mut task = running_task();

        assert_eq!(
            task.queue().unwrap_err().kind(),
            ErrorKind::InvalidTaskState
        );
    }

    #[test]
    fn terminal_tasks_cannot_start_again() {
        let mut completed = running_task();
        completed.complete(output()).unwrap();
        let mut failed = running_task();
        failed
            .fail(EngineError::new(ErrorKind::Runtime, "test"))
            .unwrap();

        assert_eq!(
            completed.start().unwrap_err().kind(),
            ErrorKind::InvalidTaskState
        );
        assert_eq!(
            failed.start().unwrap_err().kind(),
            ErrorKind::InvalidTaskState
        );
    }
}
