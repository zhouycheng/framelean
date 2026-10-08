use std::error::Error;
use std::fmt::{self, Display, Formatter};

use framelean_core::{EngineError, ErrorKind, NodeId, ProcessorId, TaskId};

use crate::{AudioFrame, MediaPacket, VideoFrame};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ProcessingStage {
    Packet,
    Video,
    Audio,
}

impl Display for ProcessingStage {
    fn fmt(&self, formatter: &mut Formatter<'_>) -> fmt::Result {
        let value = match self {
            Self::Packet => "packet",
            Self::Video => "video",
            Self::Audio => "audio",
        };
        formatter.write_str(value)
    }
}

#[derive(Debug, PartialEq, Eq)]
pub enum ProcessInput {
    Packet(MediaPacket),
    Video(VideoFrame),
    Audio(AudioFrame),
}

impl ProcessInput {
    pub fn stage(&self) -> ProcessingStage {
        match self {
            Self::Packet(_) => ProcessingStage::Packet,
            Self::Video(_) => ProcessingStage::Video,
            Self::Audio(_) => ProcessingStage::Audio,
        }
    }
}

#[derive(Debug, PartialEq, Eq)]
pub enum ProcessOutput {
    Packet(MediaPacket),
    Video(VideoFrame),
    Audio(AudioFrame),
}

impl ProcessOutput {
    pub fn stage(&self) -> ProcessingStage {
        match self {
            Self::Packet(_) => ProcessingStage::Packet,
            Self::Video(_) => ProcessingStage::Video,
            Self::Audio(_) => ProcessingStage::Audio,
        }
    }
}

impl From<ProcessOutput> for ProcessInput {
    fn from(output: ProcessOutput) -> Self {
        match output {
            ProcessOutput::Packet(value) => Self::Packet(value),
            ProcessOutput::Video(value) => Self::Video(value),
            ProcessOutput::Audio(value) => Self::Audio(value),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ProcessorMetadata {
    id: ProcessorId,
    name: String,
    stage: ProcessingStage,
}

impl ProcessorMetadata {
    pub fn new(
        id: ProcessorId,
        name: impl Into<String>,
        stage: ProcessingStage,
    ) -> ProcessorResult<Self> {
        let name = name.into();
        if name.trim().is_empty() {
            return Err(ProcessorError::InvalidMetadata(
                "processor name cannot be empty".to_owned(),
            ));
        }
        Ok(Self { id, name, stage })
    }

    pub fn id(&self) -> &ProcessorId {
        &self.id
    }

    pub fn name(&self) -> &str {
        &self.name
    }

    pub fn stage(&self) -> ProcessingStage {
        self.stage
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ProcessorContext {
    task_id: TaskId,
    node_id: NodeId,
}

impl ProcessorContext {
    pub fn new(task_id: TaskId, node_id: NodeId) -> Self {
        Self { task_id, node_id }
    }

    pub fn task_id(&self) -> &TaskId {
        &self.task_id
    }

    pub fn node_id(&self) -> &NodeId {
        &self.node_id
    }
}

#[derive(Debug, PartialEq, Eq)]
pub enum ProcessorError {
    InvalidMetadata(String),
    StageMismatch {
        expected: ProcessingStage,
        actual: ProcessingStage,
    },
    Failed(String),
}

impl ProcessorError {
    pub fn failed(message: impl Into<String>) -> Self {
        Self::Failed(message.into())
    }
}

impl Display for ProcessorError {
    fn fmt(&self, formatter: &mut Formatter<'_>) -> fmt::Result {
        match self {
            Self::InvalidMetadata(message) => write!(formatter, "invalid metadata: {message}"),
            Self::StageMismatch { expected, actual } => {
                write!(formatter, "expected {expected} data, received {actual}")
            }
            Self::Failed(message) => write!(formatter, "processing failed: {message}"),
        }
    }
}

impl Error for ProcessorError {}

impl From<ProcessorError> for EngineError {
    fn from(error: ProcessorError) -> Self {
        let message = error.to_string();
        Self::with_source(ErrorKind::Processor, message, error)
    }
}

pub type ProcessorResult<T> = std::result::Result<T, ProcessorError>;

pub trait Processor: Send {
    fn metadata(&self) -> &ProcessorMetadata;

    fn process(
        &mut self,
        input: ProcessInput,
        context: &mut ProcessorContext,
    ) -> ProcessorResult<ProcessOutput>;
}

#[cfg(test)]
mod tests {
    use framelean_core::ErrorKind;

    use super::*;
    use crate::{MediaBuffer, StreamId, VideoFrame};

    #[test]
    fn process_output_moves_into_matching_input() {
        let frame = VideoFrame::new(StreamId::new(0), None, MediaBuffer::new(vec![1, 2, 3]));
        let pointer = frame.buffer().as_ptr();

        let input: ProcessInput = ProcessOutput::Video(frame).into();
        let ProcessInput::Video(frame) = input else {
            panic!("expected video input");
        };

        assert_eq!(pointer, frame.buffer().as_ptr());
    }

    #[test]
    fn processor_error_converts_in_owning_crate() {
        let error = EngineError::from(ProcessorError::failed("test"));
        assert_eq!(error.kind(), ErrorKind::Processor);
        assert!(
            std::error::Error::source(&error)
                .unwrap()
                .downcast_ref::<ProcessorError>()
                .is_some()
        );
    }
}
