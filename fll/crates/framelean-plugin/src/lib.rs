use std::collections::HashMap;
use std::error::Error;
use std::fmt::{self, Display, Formatter};

use framelean_core::{EngineError, ErrorKind, ProcessorId};
use framelean_media::capability::{BackendCatalog, BackendCatalogProvider, BackendDescriptor};
use framelean_media::processor::{ProcessingStage, Processor, ProcessorMetadata};

pub trait ProcessorFactory: Send + Sync {
    fn metadata(&self) -> &ProcessorMetadata;
    fn create(&self) -> PluginResult<Box<dyn Processor>>;

    fn backend_capabilities(&self) -> Vec<BackendDescriptor> {
        Vec::new()
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PluginMetadata {
    id: String,
    name: String,
}

impl PluginMetadata {
    pub fn new(id: impl Into<String>, name: impl Into<String>) -> PluginResult<Self> {
        let id = id.into();
        let name = name.into();
        if id.trim().is_empty() {
            return Err(PluginError::InvalidMetadata(
                "plugin id cannot be empty".to_owned(),
            ));
        }
        if name.trim().is_empty() {
            return Err(PluginError::InvalidMetadata(
                "plugin name cannot be empty".to_owned(),
            ));
        }
        Ok(Self { id, name })
    }

    pub fn id(&self) -> &str {
        &self.id
    }

    pub fn name(&self) -> &str {
        &self.name
    }
}

pub trait Plugin {
    fn metadata(&self) -> &PluginMetadata;
    fn register(&self, registry: &mut PluginRegistry) -> PluginResult<()>;
}

pub struct PluginRegistry {
    factories: HashMap<ProcessorId, Box<dyn ProcessorFactory>>,
}

impl PluginRegistry {
    pub fn new() -> Self {
        Self {
            factories: HashMap::new(),
        }
    }

    pub fn register_factory(&mut self, factory: Box<dyn ProcessorFactory>) -> PluginResult<()> {
        let processor_id = factory.metadata().id().clone();
        if self.factories.contains_key(&processor_id) {
            return Err(PluginError::DuplicateProcessor(processor_id));
        }
        self.factories.insert(processor_id, factory);
        Ok(())
    }

    pub fn factory(&self, processor_id: &ProcessorId) -> PluginResult<&dyn ProcessorFactory> {
        self.factories
            .get(processor_id)
            .map(Box::as_ref)
            .ok_or_else(|| PluginError::ProcessorNotFound(processor_id.clone()))
    }

    pub fn len(&self) -> usize {
        self.factories.len()
    }

    pub fn is_empty(&self) -> bool {
        self.factories.is_empty()
    }
}

impl Default for PluginRegistry {
    fn default() -> Self {
        Self::new()
    }
}

impl BackendCatalogProvider for PluginRegistry {
    fn backend_catalog(&self) -> framelean_core::Result<BackendCatalog> {
        let mut backends = Vec::new();
        for factory in self.factories.values() {
            for backend in factory.backend_capabilities() {
                backend.validate()?;
                backends.push(backend);
            }
        }
        BackendCatalog::from_backends(backends)
    }
}

#[derive(Debug, PartialEq, Eq)]
pub enum PluginError {
    InvalidMetadata(String),
    DuplicateProcessor(ProcessorId),
    ProcessorNotFound(ProcessorId),
    FactoryMetadataMismatch {
        requested: ProcessorId,
        actual: ProcessorId,
    },
    ProcessorMetadataMismatch {
        requested: ProcessorId,
        actual: ProcessorId,
    },
    ProcessorStageMismatch {
        expected: ProcessingStage,
        actual: ProcessingStage,
    },
    FactoryCreation(String),
}

impl Display for PluginError {
    fn fmt(&self, formatter: &mut Formatter<'_>) -> fmt::Result {
        match self {
            Self::InvalidMetadata(message) => {
                write!(formatter, "invalid plugin metadata: {message}")
            }
            Self::DuplicateProcessor(id) => {
                write!(formatter, "processor factory {id} is already registered")
            }
            Self::ProcessorNotFound(id) => {
                write!(formatter, "processor factory {id} is not registered")
            }
            Self::FactoryMetadataMismatch { requested, actual } => write!(
                formatter,
                "factory metadata id {actual} does not match requested id {requested}"
            ),
            Self::ProcessorMetadataMismatch { requested, actual } => write!(
                formatter,
                "processor metadata id {actual} does not match requested id {requested}"
            ),
            Self::ProcessorStageMismatch { expected, actual } => write!(
                formatter,
                "processor stage {actual} does not match factory stage {expected}"
            ),
            Self::FactoryCreation(message) => {
                write!(formatter, "processor factory failed: {message}")
            }
        }
    }
}

impl Error for PluginError {}

impl From<PluginError> for EngineError {
    fn from(error: PluginError) -> Self {
        let message = error.to_string();
        Self::with_source(ErrorKind::Plugin, message, error)
    }
}

pub type PluginResult<T> = std::result::Result<T, PluginError>;

#[cfg(test)]
mod tests {
    use framelean_core::{ErrorKind, NodeId, TaskId};
    use framelean_media::processor::{
        ProcessInput, ProcessOutput, ProcessorContext, ProcessorResult,
    };

    use super::*;

    struct PassthroughProcessor {
        metadata: ProcessorMetadata,
        calls: usize,
    }

    impl Processor for PassthroughProcessor {
        fn metadata(&self) -> &ProcessorMetadata {
            &self.metadata
        }

        fn process(
            &mut self,
            input: ProcessInput,
            _context: &mut ProcessorContext,
        ) -> ProcessorResult<ProcessOutput> {
            self.calls += 1;
            Ok(match input {
                ProcessInput::Packet(value) => ProcessOutput::Packet(value),
                ProcessInput::Video(value) => ProcessOutput::Video(value),
                ProcessInput::Audio(value) => ProcessOutput::Audio(value),
            })
        }
    }

    struct PassthroughFactory {
        metadata: ProcessorMetadata,
    }

    impl ProcessorFactory for PassthroughFactory {
        fn metadata(&self) -> &ProcessorMetadata {
            &self.metadata
        }

        fn create(&self) -> PluginResult<Box<dyn Processor>> {
            Ok(Box::new(PassthroughProcessor {
                metadata: self.metadata.clone(),
                calls: 0,
            }))
        }
    }

    struct ExampleAudioDecryptPlugin {
        metadata: PluginMetadata,
        processor_metadata: ProcessorMetadata,
    }

    impl ExampleAudioDecryptPlugin {
        fn new() -> Self {
            Self {
                metadata: PluginMetadata::new(
                    "example.audio-decrypt",
                    "Example Audio Decrypt Plugin",
                )
                .unwrap(),
                processor_metadata: ProcessorMetadata::new(
                    ProcessorId::new("example.audio-decrypt.passthrough").unwrap(),
                    "Example Audio Decrypt Passthrough",
                    ProcessingStage::Audio,
                )
                .unwrap(),
            }
        }
    }

    impl Plugin for ExampleAudioDecryptPlugin {
        fn metadata(&self) -> &PluginMetadata {
            &self.metadata
        }

        fn register(&self, registry: &mut PluginRegistry) -> PluginResult<()> {
            registry.register_factory(Box::new(PassthroughFactory {
                metadata: self.processor_metadata.clone(),
            }))
        }
    }

    #[test]
    fn plugin_registers_factory_using_its_metadata_id() {
        let plugin = ExampleAudioDecryptPlugin::new();
        let processor_id = plugin.processor_metadata.id().clone();
        let mut registry = PluginRegistry::new();

        plugin.register(&mut registry).unwrap();

        assert_eq!(registry.len(), 1);
        assert_eq!(
            registry.factory(&processor_id).unwrap().metadata().id(),
            &processor_id
        );
    }

    #[test]
    fn duplicate_processor_id_is_rejected() {
        let plugin = ExampleAudioDecryptPlugin::new();
        let mut registry = PluginRegistry::new();
        plugin.register(&mut registry).unwrap();

        let error = plugin.register(&mut registry).unwrap_err();

        assert!(matches!(error, PluginError::DuplicateProcessor(_)));
    }

    #[test]
    fn factory_creates_independent_processor_instances() {
        let plugin = ExampleAudioDecryptPlugin::new();
        let mut registry = PluginRegistry::new();
        plugin.register(&mut registry).unwrap();
        let factory = registry.factory(plugin.processor_metadata.id()).unwrap();

        let first = factory.create().unwrap();
        let second = factory.create().unwrap();

        assert!(!std::ptr::eq::<dyn Processor>(&*first, &*second));
    }

    #[test]
    fn plugin_error_converts_in_owning_crate() {
        let error = EngineError::from(PluginError::FactoryCreation("test".to_owned()));
        assert_eq!(error.kind(), ErrorKind::Plugin);
        assert!(
            std::error::Error::source(&error)
                .unwrap()
                .downcast_ref::<PluginError>()
                .is_some()
        );
    }

    #[test]
    fn example_plugin_is_only_a_passthrough_registration_fixture() {
        let plugin = ExampleAudioDecryptPlugin::new();
        let mut registry = PluginRegistry::new();
        plugin.register(&mut registry).unwrap();
        let factory = registry.factory(plugin.processor_metadata.id()).unwrap();
        let mut processor = factory.create().unwrap();
        let mut context = ProcessorContext::new(
            TaskId::new("task-test").unwrap(),
            NodeId::new("node-test").unwrap(),
        );
        let input = ProcessInput::Audio(framelean_media::AudioFrame::new(
            framelean_media::StreamId::new(0),
            None,
            framelean_media::MediaBuffer::new(vec![1]),
        ));

        assert!(matches!(
            processor.process(input, &mut context).unwrap(),
            ProcessOutput::Audio(_)
        ));
    }
}
