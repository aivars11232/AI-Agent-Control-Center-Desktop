use crate::app_state::ModelDefinition;
use serde::{Deserialize, Serialize};
use std::{
    collections::BTreeMap,
    fmt,
    sync::{
        atomic::{AtomicBool, Ordering},
        Arc,
    },
};

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub(crate) enum RuntimeProviderId {
    Codex,
    Ollama,
}

impl RuntimeProviderId {
    pub(crate) const fn as_str(self) -> &'static str {
        match self {
            Self::Codex => "codex",
            Self::Ollama => "ollama",
        }
    }

    pub(crate) fn from_persisted(value: &str) -> Result<Self, ProviderError> {
        match value {
            "codex" => Ok(Self::Codex),
            "ollama" => Ok(Self::Ollama),
            _ => Err(ProviderError::new(
                ProviderErrorCode::UnsupportedProvider,
                "The active AI provider is not registered.",
                false,
            )),
        }
    }
}

impl fmt::Display for RuntimeProviderId {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(self.as_str())
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct CatalogProviderBinding {
    pub(crate) catalog_provider: String,
    pub(crate) provider_id: Option<RuntimeProviderId>,
    pub(crate) adapter_available: bool,
    pub(crate) message: String,
}

pub(crate) fn catalog_provider_bindings() -> Vec<CatalogProviderBinding> {
    ["OpenAI", "Anthropic", "Google", "Ollama", "Custom"]
        .into_iter()
        .map(catalog_provider_binding)
        .collect()
}

pub(crate) fn catalog_provider_binding(provider: &str) -> CatalogProviderBinding {
    let (provider_id, message) = match provider {
        "OpenAI" => (
            Some(RuntimeProviderId::Codex),
            "Runs through the installed Codex CLI.",
        ),
        "Ollama" => (
            Some(RuntimeProviderId::Ollama),
            "Runs through the local Ollama service.",
        ),
        "Anthropic" | "Google" | "Custom" => (
            None,
            "No executable runtime adapter is registered for this catalog provider.",
        ),
        _ => (None, "The catalog provider is not recognized."),
    };
    CatalogProviderBinding {
        catalog_provider: provider.to_string(),
        provider_id,
        adapter_available: provider_id.is_some(),
        message: message.to_string(),
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct ProviderCapabilities {
    pub(crate) workspace_read: bool,
    pub(crate) workspace_write: bool,
    pub(crate) web_search: bool,
    pub(crate) workspace_tools: bool,
    pub(crate) usage_reporting: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct ProviderDescriptor {
    pub(crate) id: RuntimeProviderId,
    pub(crate) display_name: String,
    pub(crate) capabilities: ProviderCapabilities,
}

pub(crate) fn codex_descriptor() -> ProviderDescriptor {
    ProviderDescriptor {
        id: RuntimeProviderId::Codex,
        display_name: "Codex".to_string(),
        capabilities: ProviderCapabilities {
            workspace_read: true,
            workspace_write: true,
            web_search: true,
            workspace_tools: false,
            usage_reporting: true,
        },
    }
}

pub(crate) fn ollama_descriptor() -> ProviderDescriptor {
    ProviderDescriptor {
        id: RuntimeProviderId::Ollama,
        display_name: "Ollama".to_string(),
        capabilities: ProviderCapabilities {
            workspace_read: true,
            workspace_write: true,
            web_search: false,
            workspace_tools: true,
            usage_reporting: true,
        },
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub(crate) enum ProviderAvailability {
    Ready,
    Unavailable,
    Unknown,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct ProviderRuntimeModel {
    pub(crate) name: String,
    pub(crate) capabilities: Vec<String>,
    pub(crate) context_length: Option<u64>,
    pub(crate) availability: ProviderAvailability,
    pub(crate) message: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct ProviderRuntimeStatus {
    pub(crate) provider: ProviderDescriptor,
    pub(crate) availability: ProviderAvailability,
    pub(crate) version: Option<String>,
    pub(crate) models: Vec<ProviderRuntimeModel>,
    pub(crate) message: String,
}

impl ProviderRuntimeStatus {
    pub(crate) fn unknown(provider: ProviderDescriptor, message: impl Into<String>) -> Self {
        Self {
            provider,
            availability: ProviderAvailability::Unknown,
            version: None,
            models: Vec::new(),
            message: message.into(),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct ProviderRegistrySnapshot {
    pub(crate) providers: Vec<ProviderRuntimeStatus>,
    pub(crate) catalog_bindings: Vec<CatalogProviderBinding>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct ProviderModelIdentity {
    pub(crate) catalog_model_id: i64,
    pub(crate) provider_id: RuntimeProviderId,
    pub(crate) runtime_model: String,
}

pub(crate) fn resolve_model_identity(
    models: &[ModelDefinition],
    selected_model: &str,
    active_provider: &str,
) -> Result<ProviderModelIdentity, ProviderError> {
    let active_provider = RuntimeProviderId::from_persisted(active_provider)?;
    let matches = models
        .iter()
        .filter(|model| model.name == selected_model)
        .collect::<Vec<_>>();
    let model = match matches.as_slice() {
        [] => {
            return Err(ProviderError::new(
                ProviderErrorCode::ModelNotFound,
                "The selected agent model is not registered in backend state.",
                false,
            ))
        }
        [model] => *model,
        _ => {
            return Err(ProviderError::new(
                ProviderErrorCode::ModelAmbiguous,
                "The selected agent model name is ambiguous in the model catalog.",
                false,
            ))
        }
    };
    let binding = catalog_provider_binding(&model.provider);
    let provider_id = binding.provider_id.ok_or_else(|| {
        ProviderError::new(
            ProviderErrorCode::UnsupportedProvider,
            binding.message,
            false,
        )
        .with_model(model.name.clone())
    })?;
    if provider_id != active_provider {
        return Err(ProviderError::new(
            ProviderErrorCode::ProviderModelMismatch,
            format!(
                "The selected model runs through {provider_id}, but {active_provider} is the active AI provider."
            ),
            false,
        )
        .with_provider(provider_id)
        .with_model(model.name.clone()));
    }
    Ok(ProviderModelIdentity {
        catalog_model_id: model.id,
        provider_id,
        runtime_model: model.name.clone(),
    })
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum ProviderRunMode {
    Execute,
    Review,
}

#[derive(Debug, Clone)]
pub(crate) struct ProviderRunRequest {
    pub(crate) run_mode: ProviderRunMode,
    pub(crate) agent_name: String,
    pub(crate) description: String,
    pub(crate) role: String,
    pub(crate) category: String,
    pub(crate) memory: String,
    pub(crate) review_feedback: Option<String>,
    pub(crate) task_title: String,
    pub(crate) model: ProviderModelIdentity,
    pub(crate) strength: u8,
    pub(crate) focus: String,
    pub(crate) enable_web_search: bool,
    pub(crate) workspace_path: String,
    pub(crate) file_access: String,
    pub(crate) terminal_access: String,
    pub(crate) authorized_scopes: Vec<String>,
    pub(crate) destructive_actions_approved: bool,
    pub(crate) timeout_seconds: u64,
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub(crate) struct ProviderRunUsage {
    pub(crate) input_tokens: Option<u64>,
    pub(crate) output_tokens: Option<u64>,
    pub(crate) total_tokens: Option<u64>,
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub(crate) struct ProviderRunEvidence {
    pub(crate) stderr_excerpt: Option<String>,
    pub(crate) stdout_truncated: bool,
    pub(crate) stderr_truncated: bool,
    pub(crate) diff_truncated: bool,
    pub(crate) changed_files_truncated: bool,
    pub(crate) before_snapshot_truncated: bool,
    pub(crate) after_snapshot_truncated: bool,
    pub(crate) original_stdout_bytes: u64,
    pub(crate) original_stderr_bytes: u64,
    pub(crate) original_diff_bytes: u64,
    pub(crate) original_changed_file_count: u64,
}

#[derive(Debug, Clone, PartialEq)]
pub(crate) struct ProviderRunResult {
    pub(crate) provider_id: RuntimeProviderId,
    pub(crate) output: String,
    pub(crate) response_id: Option<String>,
    pub(crate) model: String,
    pub(crate) usage: ProviderRunUsage,
    pub(crate) changed_files: Vec<String>,
    pub(crate) diff: Option<String>,
    pub(crate) duration_seconds: u64,
    pub(crate) evidence: ProviderRunEvidence,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum ProviderEventKind {
    Status,
    Progress,
    Complete,
}

impl ProviderEventKind {
    pub(crate) const fn as_str(self) -> &'static str {
        match self {
            Self::Status => "status",
            Self::Progress => "progress",
            Self::Complete => "complete",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct ProviderRunEvent {
    pub(crate) kind: ProviderEventKind,
    pub(crate) message: String,
}

pub(crate) trait ProviderRunObserver: Send + Sync {
    fn emit(&self, event: ProviderRunEvent) -> Result<(), ProviderError>;
    fn mark_started(&self) -> Result<(), ProviderError>;
}

#[derive(Clone)]
pub(crate) struct ProviderCancellation {
    flag: Arc<AtomicBool>,
}

impl ProviderCancellation {
    pub(crate) fn new(flag: Arc<AtomicBool>) -> Self {
        Self { flag }
    }

    pub(crate) fn is_cancelled(&self) -> bool {
        self.flag.load(Ordering::SeqCst)
    }
}

#[derive(Clone)]
pub(crate) struct ProviderRunContext {
    observer: Arc<dyn ProviderRunObserver>,
    cancellation: ProviderCancellation,
}

impl ProviderRunContext {
    pub(crate) fn new(
        observer: Arc<dyn ProviderRunObserver>,
        cancellation: ProviderCancellation,
    ) -> Self {
        Self {
            observer,
            cancellation,
        }
    }

    pub(crate) fn emit(
        &self,
        kind: ProviderEventKind,
        message: impl Into<String>,
    ) -> Result<(), ProviderError> {
        self.observer.emit(ProviderRunEvent {
            kind,
            message: message.into(),
        })
    }

    pub(crate) fn mark_started(&self) -> Result<(), ProviderError> {
        self.observer.mark_started()
    }

    pub(crate) fn is_cancelled(&self) -> bool {
        self.cancellation.is_cancelled()
    }

    pub(crate) fn cancellation(&self) -> ProviderCancellation {
        self.cancellation.clone()
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum ProviderErrorCode {
    UnsupportedProvider,
    ProviderModelMismatch,
    ModelNotFound,
    ModelAmbiguous,
    ProviderUnavailable,
    ModelUnavailable,
    CapabilityUnsupported,
    RuntimeIncompatible,
    Cancelled,
    TimedOut,
    StartupFailed,
    ExecutionFailed,
    OutputLimitExceeded,
    CleanupFailed,
    ProtocolError,
    EventSinkFailed,
}

impl ProviderErrorCode {
    pub(crate) const fn as_str(self) -> &'static str {
        match self {
            Self::UnsupportedProvider => "UNSUPPORTED_PROVIDER",
            Self::ProviderModelMismatch => "PROVIDER_MODEL_MISMATCH",
            Self::ModelNotFound => "MODEL_NOT_FOUND",
            Self::ModelAmbiguous => "MODEL_AMBIGUOUS",
            Self::ProviderUnavailable => "PROVIDER_UNAVAILABLE",
            Self::ModelUnavailable => "MODEL_UNAVAILABLE",
            Self::CapabilityUnsupported => "CAPABILITY_UNSUPPORTED",
            Self::RuntimeIncompatible => "PROVIDER_RUNTIME_INCOMPATIBLE",
            Self::Cancelled => "PROVIDER_CANCELLED",
            Self::TimedOut => "PROVIDER_TIMED_OUT",
            Self::StartupFailed => "PROVIDER_START_FAILED",
            Self::ExecutionFailed => "PROVIDER_EXECUTION_FAILED",
            Self::OutputLimitExceeded => "PROVIDER_OUTPUT_LIMIT_EXCEEDED",
            Self::CleanupFailed => "PROVIDER_CLEANUP_FAILED",
            Self::ProtocolError => "PROVIDER_PROTOCOL_ERROR",
            Self::EventSinkFailed => "PROVIDER_EVENT_SINK_FAILED",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct ProviderError {
    pub(crate) code: ProviderErrorCode,
    pub(crate) message: String,
    pub(crate) provider_id: Option<RuntimeProviderId>,
    pub(crate) model: Option<String>,
    pub(crate) retryable: bool,
    pub(crate) evidence: ProviderRunEvidence,
}

impl ProviderError {
    pub(crate) fn new(
        code: ProviderErrorCode,
        message: impl Into<String>,
        retryable: bool,
    ) -> Self {
        Self {
            code,
            message: message.into(),
            provider_id: None,
            model: None,
            retryable,
            evidence: ProviderRunEvidence::default(),
        }
    }

    pub(crate) fn with_provider(mut self, provider_id: RuntimeProviderId) -> Self {
        self.provider_id = Some(provider_id);
        self
    }

    pub(crate) fn with_model(mut self, model: impl Into<String>) -> Self {
        self.model = Some(model.into());
        self
    }

    pub(crate) fn with_evidence(mut self, evidence: ProviderRunEvidence) -> Self {
        self.evidence = evidence;
        self
    }
}

impl fmt::Display for ProviderError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(formatter, "{}: {}", self.code.as_str(), self.message)
    }
}

impl std::error::Error for ProviderError {}

pub(crate) trait ProviderAdapter: Send + Sync {
    fn descriptor(&self) -> ProviderDescriptor;
    fn inspect(&self) -> ProviderRuntimeStatus;
    fn run(
        &self,
        context: ProviderRunContext,
        request: ProviderRunRequest,
    ) -> Result<ProviderRunResult, ProviderError>;
}

pub(crate) struct ProviderRegistry {
    adapters: BTreeMap<RuntimeProviderId, Arc<dyn ProviderAdapter>>,
}

impl ProviderRegistry {
    pub(crate) fn new(
        adapters: impl IntoIterator<Item = Arc<dyn ProviderAdapter>>,
    ) -> Result<Self, ProviderError> {
        let mut registered = BTreeMap::new();
        for adapter in adapters {
            let provider_id = adapter.descriptor().id;
            if registered.insert(provider_id, adapter).is_some() {
                return Err(ProviderError::new(
                    ProviderErrorCode::StartupFailed,
                    format!("The provider registry contains duplicate `{provider_id}` adapters."),
                    false,
                )
                .with_provider(provider_id));
            }
        }
        Ok(Self {
            adapters: registered,
        })
    }

    pub(crate) fn snapshot(&self) -> ProviderRegistrySnapshot {
        ProviderRegistrySnapshot {
            providers: self
                .adapters
                .values()
                .map(|adapter| adapter.inspect())
                .collect(),
            catalog_bindings: catalog_provider_bindings(),
        }
    }

    #[cfg(test)]
    pub(crate) fn provider_ids(&self) -> Vec<RuntimeProviderId> {
        self.adapters.keys().copied().collect()
    }

    pub(crate) fn run(
        &self,
        provider_id: RuntimeProviderId,
        context: ProviderRunContext,
        request: ProviderRunRequest,
    ) -> Result<ProviderRunResult, ProviderError> {
        if request.model.provider_id != provider_id {
            return Err(ProviderError::new(
                ProviderErrorCode::ProviderModelMismatch,
                format!(
                    "The resolved model belongs to {}, not {provider_id}.",
                    request.model.provider_id
                ),
                false,
            )
            .with_provider(provider_id)
            .with_model(request.model.runtime_model));
        }
        let adapter = self.adapters.get(&provider_id).ok_or_else(|| {
            ProviderError::new(
                ProviderErrorCode::UnsupportedProvider,
                format!("No executable `{provider_id}` runtime adapter is registered."),
                false,
            )
            .with_provider(provider_id)
        })?;
        adapter.run(context, request)
    }
}

#[cfg(test)]
pub(crate) mod test_support {
    use super::*;
    use std::sync::Mutex;

    #[derive(Default)]
    pub(crate) struct RecordingObserver {
        entries: Mutex<Vec<String>>,
    }

    impl RecordingObserver {
        pub(crate) fn entries(&self) -> Vec<String> {
            self.entries.lock().unwrap().clone()
        }
    }

    impl ProviderRunObserver for RecordingObserver {
        fn emit(&self, event: ProviderRunEvent) -> Result<(), ProviderError> {
            self.entries
                .lock()
                .unwrap()
                .push(format!("{}:{}", event.kind.as_str(), event.message));
            Ok(())
        }

        fn mark_started(&self) -> Result<(), ProviderError> {
            self.entries.lock().unwrap().push("started".to_string());
            Ok(())
        }
    }

    pub(crate) struct FakeAdapter {
        descriptor: ProviderDescriptor,
        status: ProviderRuntimeStatus,
        invocations: Arc<Mutex<Vec<ProviderModelIdentity>>>,
    }

    impl FakeAdapter {
        pub(crate) fn new(
            descriptor: ProviderDescriptor,
            invocations: Arc<Mutex<Vec<ProviderModelIdentity>>>,
        ) -> Self {
            let status = ProviderRuntimeStatus {
                provider: descriptor.clone(),
                availability: ProviderAvailability::Ready,
                version: Some("fake-1".to_string()),
                models: Vec::new(),
                message: "Fake provider ready.".to_string(),
            };
            Self {
                descriptor,
                status,
                invocations,
            }
        }
    }

    impl ProviderAdapter for FakeAdapter {
        fn descriptor(&self) -> ProviderDescriptor {
            self.descriptor.clone()
        }

        fn inspect(&self) -> ProviderRuntimeStatus {
            self.status.clone()
        }

        fn run(
            &self,
            context: ProviderRunContext,
            request: ProviderRunRequest,
        ) -> Result<ProviderRunResult, ProviderError> {
            if context.is_cancelled() {
                return Err(ProviderError::new(
                    ProviderErrorCode::Cancelled,
                    "Fake provider run cancelled.",
                    true,
                )
                .with_provider(self.descriptor.id));
            }
            context.emit(ProviderEventKind::Status, "accepted")?;
            context.mark_started()?;
            context.emit(ProviderEventKind::Progress, "working")?;
            self.invocations.lock().unwrap().push(request.model.clone());
            context.emit(ProviderEventKind::Complete, "complete")?;
            Ok(ProviderRunResult {
                provider_id: self.descriptor.id,
                output: "fake output".to_string(),
                response_id: Some("fake-response".to_string()),
                model: request.model.runtime_model,
                usage: ProviderRunUsage {
                    input_tokens: Some(2),
                    output_tokens: Some(3),
                    total_tokens: Some(5),
                },
                changed_files: Vec::new(),
                diff: None,
                duration_seconds: 1,
                evidence: ProviderRunEvidence::default(),
            })
        }
    }

    pub(crate) fn request(provider_id: RuntimeProviderId) -> ProviderRunRequest {
        ProviderRunRequest {
            run_mode: ProviderRunMode::Execute,
            agent_name: "Fixture Agent".to_string(),
            description: "Provider contract fixture".to_string(),
            role: "Specialist".to_string(),
            category: "Development".to_string(),
            memory: String::new(),
            review_feedback: None,
            task_title: "Inspect the workspace".to_string(),
            model: ProviderModelIdentity {
                catalog_model_id: 1,
                provider_id,
                runtime_model: "fixture-model".to_string(),
            },
            strength: 5,
            focus: "balanced".to_string(),
            enable_web_search: false,
            workspace_path: "/tmp/task-0006-fixture".to_string(),
            file_access: "read".to_string(),
            terminal_access: "none".to_string(),
            authorized_scopes: Vec::new(),
            destructive_actions_approved: false,
            timeout_seconds: 60,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::test_support::{request, FakeAdapter, RecordingObserver};
    use super::*;
    use std::sync::Mutex;

    fn model(id: i64, name: &str, provider: &str) -> ModelDefinition {
        ModelDefinition {
            id,
            name: name.to_string(),
            provider: provider.to_string(),
        }
    }

    #[test]
    fn task_0006_model_resolution_is_exact_supported_and_active() {
        let models = vec![
            model(1, "gpt-fixture", "OpenAI"),
            model(2, "ollama-fixture", "Ollama"),
            model(3, "claude-fixture", "Anthropic"),
        ];
        assert_eq!(
            resolve_model_identity(&models, "gpt-fixture", "codex").unwrap(),
            ProviderModelIdentity {
                catalog_model_id: 1,
                provider_id: RuntimeProviderId::Codex,
                runtime_model: "gpt-fixture".to_string(),
            }
        );
        assert_eq!(
            resolve_model_identity(&models, "missing", "codex")
                .unwrap_err()
                .code,
            ProviderErrorCode::ModelNotFound
        );
        assert_eq!(
            resolve_model_identity(&models, "claude-fixture", "codex")
                .unwrap_err()
                .code,
            ProviderErrorCode::UnsupportedProvider
        );
        assert_eq!(
            resolve_model_identity(&models, "ollama-fixture", "codex")
                .unwrap_err()
                .code,
            ProviderErrorCode::ProviderModelMismatch
        );
    }

    #[test]
    fn task_0006_duplicate_model_names_fail_closed() {
        let models = vec![
            model(1, "duplicate", "OpenAI"),
            model(2, "duplicate", "Ollama"),
        ];
        assert_eq!(
            resolve_model_identity(&models, "duplicate", "codex")
                .unwrap_err()
                .code,
            ProviderErrorCode::ModelAmbiguous
        );
    }

    #[test]
    fn task_0006_registry_dispatches_exactly_one_adapter_without_fallback() {
        let codex_invocations = Arc::new(Mutex::new(Vec::new()));
        let ollama_invocations = Arc::new(Mutex::new(Vec::new()));
        let registry = ProviderRegistry::new([
            Arc::new(FakeAdapter::new(
                codex_descriptor(),
                codex_invocations.clone(),
            )) as Arc<dyn ProviderAdapter>,
            Arc::new(FakeAdapter::new(
                ollama_descriptor(),
                ollama_invocations.clone(),
            )) as Arc<dyn ProviderAdapter>,
        ])
        .unwrap();
        let observer = Arc::new(RecordingObserver::default());
        let context = ProviderRunContext::new(
            observer.clone(),
            ProviderCancellation::new(Arc::new(AtomicBool::new(false))),
        );
        let result = registry
            .run(
                RuntimeProviderId::Ollama,
                context,
                request(RuntimeProviderId::Ollama),
            )
            .unwrap();

        assert_eq!(result.provider_id, RuntimeProviderId::Ollama);
        assert!(codex_invocations.lock().unwrap().is_empty());
        assert_eq!(ollama_invocations.lock().unwrap().len(), 1);
        assert_eq!(
            observer.entries(),
            vec![
                "status:accepted",
                "started",
                "progress:working",
                "complete:complete",
            ]
        );

        let mismatch = registry
            .run(
                RuntimeProviderId::Codex,
                ProviderRunContext::new(
                    Arc::new(RecordingObserver::default()),
                    ProviderCancellation::new(Arc::new(AtomicBool::new(false))),
                ),
                request(RuntimeProviderId::Ollama),
            )
            .unwrap_err();
        assert_eq!(mismatch.code, ProviderErrorCode::ProviderModelMismatch);
        assert!(codex_invocations.lock().unwrap().is_empty());
    }

    #[test]
    fn task_0006_registry_rejects_duplicates_and_fake_cancellation_is_typed() {
        let invocations = Arc::new(Mutex::new(Vec::new()));
        let duplicate = ProviderRegistry::new([
            Arc::new(FakeAdapter::new(codex_descriptor(), invocations.clone()))
                as Arc<dyn ProviderAdapter>,
            Arc::new(FakeAdapter::new(codex_descriptor(), invocations.clone()))
                as Arc<dyn ProviderAdapter>,
        ])
        .err()
        .expect("duplicate provider ids must fail");
        assert_eq!(duplicate.code, ProviderErrorCode::StartupFailed);

        let registry =
            ProviderRegistry::new([Arc::new(FakeAdapter::new(codex_descriptor(), invocations))
                as Arc<dyn ProviderAdapter>])
            .unwrap();
        let cancelled = Arc::new(AtomicBool::new(true));
        let error = registry
            .run(
                RuntimeProviderId::Codex,
                ProviderRunContext::new(
                    Arc::new(RecordingObserver::default()),
                    ProviderCancellation::new(cancelled),
                ),
                request(RuntimeProviderId::Codex),
            )
            .unwrap_err();
        assert_eq!(error.code, ProviderErrorCode::Cancelled);
    }

    #[test]
    fn task_0006_snapshot_exposes_only_real_adapters_and_explicit_bindings() {
        let registry = ProviderRegistry::new([
            Arc::new(FakeAdapter::new(
                codex_descriptor(),
                Arc::new(Mutex::new(Vec::new())),
            )) as Arc<dyn ProviderAdapter>,
            Arc::new(FakeAdapter::new(
                ollama_descriptor(),
                Arc::new(Mutex::new(Vec::new())),
            )) as Arc<dyn ProviderAdapter>,
        ])
        .unwrap();
        let snapshot = registry.snapshot();
        assert_eq!(snapshot.providers.len(), 2);
        assert_eq!(snapshot.providers[0].provider.id, RuntimeProviderId::Codex);
        assert_eq!(snapshot.providers[1].provider.id, RuntimeProviderId::Ollama);
        assert_eq!(snapshot.catalog_bindings.len(), 5);
        for unsupported in ["Anthropic", "Google", "Custom"] {
            let binding = snapshot
                .catalog_bindings
                .iter()
                .find(|binding| binding.catalog_provider == unsupported)
                .unwrap();
            assert!(!binding.adapter_available);
            assert_eq!(binding.provider_id, None);
        }
    }
}
