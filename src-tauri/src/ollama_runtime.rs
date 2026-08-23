use crate::{
    provider_runtime::{ProviderAvailability, ProviderCancellation},
    run_coordinator::{MAX_OLLAMA_CONVERSATION_BYTES, MAX_OLLAMA_RESPONSE_BYTES},
};
use reqwest::{
    header::{ACCEPT, CONNECTION, CONTENT_LENGTH, CONTENT_TYPE},
    redirect, Method, StatusCode, Url,
};
use serde::Serialize;
use serde_json::{json, Map, Value};
use std::{
    collections::HashSet,
    fmt,
    future::pending,
    net::IpAddr,
    time::{Duration, Instant},
};
use tokio::{runtime::Runtime, task::JoinSet};

pub(crate) const OLLAMA_DISPLAY_ENDPOINT: &str = "http://localhost:11434";
const OLLAMA_LOOPBACK_ENDPOINT: &str = "http://127.0.0.1:11434/";
const OLLAMA_CONNECT_TIMEOUT: Duration = Duration::from_secs(5);
const OLLAMA_INSPECTION_TIMEOUT: Duration = Duration::from_secs(15);
const OLLAMA_CANCEL_POLL_INTERVAL: Duration = Duration::from_millis(20);
const MAX_OLLAMA_MODELS: usize = 256;
const MAX_OLLAMA_MODEL_NAME_BYTES: usize = 512;
const MAX_OLLAMA_CONTEXT_TOKENS: u64 = 16 * 1024 * 1024;
const MAX_PARALLEL_SHOW_REQUESTS: usize = 4;
const MAX_ERROR_DETAIL_CHARS: usize = 1_024;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum OllamaErrorKind {
    Unavailable,
    ModelUnavailable,
    Cancelled,
    TimedOut,
    OutputLimit,
    Protocol,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct OllamaError {
    pub(crate) kind: OllamaErrorKind,
    pub(crate) message: String,
}

impl OllamaError {
    fn new(kind: OllamaErrorKind, message: impl Into<String>) -> Self {
        Self {
            kind,
            message: message.into(),
        }
    }

    fn unavailable(message: impl Into<String>) -> Self {
        Self::new(OllamaErrorKind::Unavailable, message)
    }

    fn protocol(message: impl Into<String>) -> Self {
        Self::new(OllamaErrorKind::Protocol, message)
    }

    fn output_limit(message: impl Into<String>) -> Self {
        Self::new(OllamaErrorKind::OutputLimit, message)
    }

    fn cancelled() -> Self {
        Self::new(
            OllamaErrorKind::Cancelled,
            "Agent run cancelled by the user.",
        )
    }

    fn timed_out() -> Self {
        Self::new(
            OllamaErrorKind::TimedOut,
            "The Ollama request reached the task deadline.",
        )
    }

    fn model_unavailable(message: impl Into<String>) -> Self {
        Self::new(OllamaErrorKind::ModelUnavailable, message)
    }
}

impl fmt::Display for OllamaError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(&self.message)
    }
}

impl std::error::Error for OllamaError {}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct OllamaRuntimeStatus {
    pub(crate) connected: bool,
    pub(crate) version: Option<String>,
    pub(crate) endpoint: String,
    pub(crate) models: Vec<OllamaModel>,
    pub(crate) message: String,
    #[serde(skip)]
    pub(crate) catalog_ready: bool,
}

impl OllamaRuntimeStatus {
    fn disconnected(endpoint: String, message: impl Into<String>) -> Self {
        Self {
            connected: false,
            version: None,
            endpoint,
            models: Vec::new(),
            message: message.into(),
            catalog_ready: false,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct OllamaModel {
    pub(crate) name: String,
    pub(crate) capabilities: Vec<String>,
    pub(crate) context_length: Option<u64>,
    pub(crate) availability: ProviderAvailability,
    pub(crate) message: String,
}

impl OllamaModel {
    fn unavailable(name: String, message: impl Into<String>) -> Self {
        Self {
            name,
            capabilities: Vec::new(),
            context_length: None,
            availability: ProviderAvailability::Unavailable,
            message: message.into(),
        }
    }

    pub(crate) fn supports_tools(&self) -> bool {
        self.availability == ProviderAvailability::Ready
            && self
                .capabilities
                .iter()
                .any(|capability| capability == "tools")
    }
}

#[derive(Clone)]
struct RequestControl {
    cancellation: Option<ProviderCancellation>,
    deadline: Instant,
}

impl RequestControl {
    fn inspection() -> Self {
        Self {
            cancellation: None,
            deadline: Instant::now() + OLLAMA_INSPECTION_TIMEOUT,
        }
    }

    fn run(cancellation: ProviderCancellation, deadline: Instant) -> Self {
        Self {
            cancellation: Some(cancellation),
            deadline,
        }
    }

    fn remaining(&self) -> Result<Duration, OllamaError> {
        self.deadline
            .checked_duration_since(Instant::now())
            .filter(|remaining| !remaining.is_zero())
            .ok_or_else(OllamaError::timed_out)
    }
}

#[derive(Clone)]
struct OllamaHttpClient {
    base_url: Url,
    display_endpoint: String,
    client: reqwest::Client,
}

impl OllamaHttpClient {
    fn new(endpoint: &str, display_endpoint: impl Into<String>) -> Result<Self, OllamaError> {
        let mut base_url = Url::parse(endpoint)
            .map_err(|error| OllamaError::protocol(format!("Invalid Ollama endpoint: {error}")))?;
        validate_loopback_endpoint(&base_url)?;
        if !base_url.path().ends_with('/') {
            base_url.set_path(&format!("{}/", base_url.path()));
        }
        let client = reqwest::Client::builder()
            .connect_timeout(OLLAMA_CONNECT_TIMEOUT)
            .redirect(redirect::Policy::none())
            .retry(reqwest::retry::never())
            .no_proxy()
            .referer(false)
            .http1_only()
            .pool_max_idle_per_host(0)
            .user_agent(concat!(
                "ai-agent-control-center/",
                env!("CARGO_PKG_VERSION")
            ))
            .build()
            .map_err(|error| {
                OllamaError::unavailable(format!("Could not create the Ollama client: {error}"))
            })?;
        Ok(Self {
            base_url,
            display_endpoint: display_endpoint.into(),
            client,
        })
    }

    async fn request_json(
        &self,
        method: Method,
        path: &'static str,
        body: Option<Value>,
        control: RequestControl,
    ) -> Result<Value, OllamaError> {
        if !path.starts_with("/api/") || path.contains(['?', '#']) {
            return Err(OllamaError::protocol(
                "The Ollama API path is outside the fixed local API boundary.",
            ));
        }
        if control
            .cancellation
            .as_ref()
            .is_some_and(ProviderCancellation::is_cancelled)
        {
            return Err(OllamaError::cancelled());
        }
        let remaining = control.remaining()?;
        let url = self
            .base_url
            .join(path.trim_start_matches('/'))
            .map_err(|error| {
                OllamaError::protocol(format!("Could not construct the Ollama URL: {error}"))
            })?;
        validate_loopback_endpoint(&url)?;

        let mut request = self
            .client
            .request(method, url)
            .header(ACCEPT, "application/json")
            .header(CONNECTION, "close")
            .timeout(remaining);
        if let Some(body) = body {
            let body = serde_json::to_vec(&body).map_err(|error| {
                OllamaError::protocol(format!("Could not encode the Ollama request: {error}"))
            })?;
            if body.len() > MAX_OLLAMA_CONVERSATION_BYTES {
                return Err(OllamaError::output_limit(format!(
                    "The Ollama request exceeded the {MAX_OLLAMA_CONVERSATION_BYTES}-byte conversation bound."
                )));
            }
            request = request.header(CONTENT_TYPE, "application/json").body(body);
        }

        let request_future = async move {
            let mut response = request.send().await.map_err(map_reqwest_error)?;
            let status = response.status();
            if response
                .headers()
                .get(CONTENT_LENGTH)
                .and_then(|value| value.to_str().ok())
                .and_then(|value| value.parse::<usize>().ok())
                .is_some_and(|length| length > MAX_OLLAMA_RESPONSE_BYTES)
            {
                return Err(OllamaError::output_limit(format!(
                    "Ollama's response exceeded the {MAX_OLLAMA_RESPONSE_BYTES}-byte response bound."
                )));
            }
            if response
                .headers()
                .get(CONTENT_TYPE)
                .and_then(|value| value.to_str().ok())
                .is_some_and(|value| !value.to_ascii_lowercase().starts_with("application/json"))
            {
                return Err(OllamaError::protocol(
                    "Ollama returned a non-JSON content type.",
                ));
            }

            let mut response_body = Vec::new();
            while let Some(chunk) = response.chunk().await.map_err(map_reqwest_error)? {
                if response_body.len().saturating_add(chunk.len()) > MAX_OLLAMA_RESPONSE_BYTES {
                    return Err(OllamaError::output_limit(format!(
                        "Ollama's decoded response exceeded the {MAX_OLLAMA_RESPONSE_BYTES}-byte response bound."
                    )));
                }
                response_body.extend_from_slice(&chunk);
            }
            let payload: Value = serde_json::from_slice(&response_body).map_err(|error| {
                OllamaError::protocol(format!("Ollama returned invalid JSON: {error}"))
            })?;
            if !status.is_success() {
                let detail = payload
                    .get("error")
                    .and_then(Value::as_str)
                    .map(bounded_error_detail)
                    .filter(|detail| !detail.is_empty())
                    .unwrap_or_else(|| "The local Ollama request failed.".to_string());
                let kind = if status == StatusCode::NOT_FOUND {
                    OllamaErrorKind::ModelUnavailable
                } else {
                    OllamaErrorKind::Protocol
                };
                return Err(OllamaError::new(
                    kind,
                    format!("Ollama returned HTTP {}: {detail}", status.as_u16()),
                ));
            }
            Ok(payload)
        };

        let mut request_task = tokio::spawn(request_future);
        tokio::select! {
            biased;
            _ = wait_for_cancellation(control.cancellation.clone()) => {
                request_task.abort();
                let _ = request_task.await;
                Err(OllamaError::cancelled())
            },
            _ = tokio::time::sleep(remaining) => {
                request_task.abort();
                let _ = request_task.await;
                Err(OllamaError::timed_out())
            },
            result = &mut request_task => result.map_err(|error| {
                OllamaError::unavailable(format!("The Ollama request task failed: {error}"))
            })?,
        }
    }
}

pub(crate) struct OllamaSession {
    runtime: Runtime,
    client: OllamaHttpClient,
}

impl OllamaSession {
    pub(crate) fn production() -> Result<Self, OllamaError> {
        Self::new(OLLAMA_LOOPBACK_ENDPOINT, OLLAMA_DISPLAY_ENDPOINT)
    }

    fn new(endpoint: &str, display_endpoint: impl Into<String>) -> Result<Self, OllamaError> {
        let runtime = tokio::runtime::Builder::new_current_thread()
            .enable_io()
            .enable_time()
            .build()
            .map_err(|error| {
                OllamaError::unavailable(format!("Could not start the Ollama runtime: {error}"))
            })?;
        let client = OllamaHttpClient::new(endpoint, display_endpoint)?;
        Ok(Self { runtime, client })
    }

    #[cfg(test)]
    pub(crate) fn for_test_endpoint(endpoint: &str) -> Result<Self, OllamaError> {
        Self::new(endpoint, endpoint.trim_end_matches('/'))
    }

    pub(crate) fn inspect_catalog(&self) -> OllamaRuntimeStatus {
        self.runtime.block_on(inspect_catalog_async(
            self.client.clone(),
            RequestControl::inspection(),
        ))
    }

    pub(crate) fn resolve_installed_model(
        &self,
        requested_model: &str,
        cancellation: ProviderCancellation,
        deadline: Instant,
    ) -> Result<OllamaModel, OllamaError> {
        self.runtime.block_on(resolve_installed_model_async(
            self.client.clone(),
            requested_model,
            RequestControl::run(cancellation, deadline),
        ))
    }

    pub(crate) fn chat(
        &self,
        model: &str,
        messages: &[Value],
        tools: &[Value],
        context_length: Option<u64>,
        cancellation: ProviderCancellation,
        deadline: Instant,
    ) -> Result<Value, OllamaError> {
        let mut request = json!({
            "model": model,
            "messages": messages,
            "tools": tools,
            "stream": false,
        });
        if let Some(context_length) = context_length {
            request["options"] = json!({ "num_ctx": context_length.min(8_192) });
        }
        self.runtime.block_on(self.client.request_json(
            Method::POST,
            "/api/chat",
            Some(request),
            RequestControl::run(cancellation, deadline),
        ))
    }
}

pub(crate) fn inspect_ollama_runtime() -> OllamaRuntimeStatus {
    match OllamaSession::production() {
        Ok(session) => session.inspect_catalog(),
        Err(error) => {
            OllamaRuntimeStatus::disconnected(OLLAMA_DISPLAY_ENDPOINT.to_string(), error.message)
        }
    }
}

async fn inspect_catalog_async(
    client: OllamaHttpClient,
    control: RequestControl,
) -> OllamaRuntimeStatus {
    let endpoint = client.display_endpoint.clone();
    let version_response = match client
        .request_json(Method::GET, "/api/version", None, control.clone())
        .await
    {
        Ok(response) => response,
        Err(error) => return OllamaRuntimeStatus::disconnected(endpoint, error.message),
    };
    let version = version_response
        .get("version")
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|version| !version.is_empty())
        .map(str::to_string);

    let tags_response = match client
        .request_json(Method::GET, "/api/tags", None, control.clone())
        .await
    {
        Ok(response) => response,
        Err(error) => {
            return OllamaRuntimeStatus {
                connected: true,
                version,
                endpoint,
                models: Vec::new(),
                message: format!(
                    "Ollama is reachable, but its installed-model catalog is unavailable: {}",
                    error.message
                ),
                catalog_ready: false,
            }
        }
    };
    let model_names = match parse_tag_names(&tags_response) {
        Ok(names) => names,
        Err(error) => {
            return OllamaRuntimeStatus {
                connected: true,
                version,
                endpoint,
                models: Vec::new(),
                message: error.message,
                catalog_ready: false,
            }
        }
    };

    let mut join_set = JoinSet::new();
    let mut next_model = 0_usize;
    while next_model < model_names.len() && join_set.len() < MAX_PARALLEL_SHOW_REQUESTS {
        spawn_show_request(
            &mut join_set,
            client.clone(),
            control.clone(),
            next_model,
            model_names[next_model].clone(),
        );
        next_model += 1;
    }
    let mut results = vec![None; model_names.len()];
    while let Some(result) = join_set.join_next().await {
        if let Ok((index, model)) = result {
            results[index] = Some(model);
        }
        if next_model < model_names.len() {
            spawn_show_request(
                &mut join_set,
                client.clone(),
                control.clone(),
                next_model,
                model_names[next_model].clone(),
            );
            next_model += 1;
        }
    }
    let models = results
        .into_iter()
        .enumerate()
        .map(|(index, model)| {
            model.unwrap_or_else(|| {
                OllamaModel::unavailable(
                    model_names[index].clone(),
                    "Ollama returned no model metadata.",
                )
            })
        })
        .collect::<Vec<_>>();
    let unavailable_count = models
        .iter()
        .filter(|model| model.availability != ProviderAvailability::Ready)
        .count();
    let model_count = models.len();
    let message = if unavailable_count == 0 {
        format!(
            "Ollama is running locally with {model_count} inspected model{}.",
            if model_count == 1 { "" } else { "s" }
        )
    } else {
        format!(
            "Ollama is running locally with {model_count} installed model{}; metadata is unavailable for {unavailable_count}.",
            if model_count == 1 { "" } else { "s" }
        )
    };
    OllamaRuntimeStatus {
        connected: true,
        version,
        endpoint,
        models,
        message,
        catalog_ready: true,
    }
}

fn spawn_show_request(
    join_set: &mut JoinSet<(usize, OllamaModel)>,
    client: OllamaHttpClient,
    control: RequestControl,
    index: usize,
    name: String,
) {
    join_set.spawn(async move {
        let model = match show_model_async(client, &name, control).await {
            Ok(model) => model,
            Err(error) => OllamaModel::unavailable(name, error.message),
        };
        (index, model)
    });
}

async fn resolve_installed_model_async(
    client: OllamaHttpClient,
    requested_model: &str,
    control: RequestControl,
) -> Result<OllamaModel, OllamaError> {
    let requested_model = normalize_model_name(requested_model)?;
    let tags = client
        .request_json(Method::GET, "/api/tags", None, control.clone())
        .await?;
    let matches = parse_tag_names(&tags)?
        .into_iter()
        .filter(|name| name.eq_ignore_ascii_case(&requested_model))
        .collect::<Vec<_>>();
    let runtime_name = match matches.as_slice() {
        [] => {
            return Err(OllamaError::model_unavailable(format!(
            "The Ollama model `{requested_model}` is not installed at {OLLAMA_DISPLAY_ENDPOINT}."
        )))
        }
        [name] => name.clone(),
        _ => {
            return Err(OllamaError::protocol(format!(
                "The Ollama model `{requested_model}` is ambiguous in local discovery."
            )))
        }
    };
    show_model_async(client, &runtime_name, control).await
}

async fn show_model_async(
    client: OllamaHttpClient,
    name: &str,
    control: RequestControl,
) -> Result<OllamaModel, OllamaError> {
    let response = client
        .request_json(
            Method::POST,
            "/api/show",
            Some(json!({ "model": name, "verbose": false })),
            control,
        )
        .await?;
    parse_show_model(name, &response)
}

fn parse_tag_names(response: &Value) -> Result<Vec<String>, OllamaError> {
    let models = response
        .get("models")
        .and_then(Value::as_array)
        .ok_or_else(|| OllamaError::protocol("Ollama's model catalog has no `models` array."))?;
    if models.len() > MAX_OLLAMA_MODELS {
        return Err(OllamaError::output_limit(format!(
            "Ollama returned more than the {MAX_OLLAMA_MODELS}-model discovery bound."
        )));
    }
    let mut names = Vec::with_capacity(models.len());
    let mut normalized_names = HashSet::with_capacity(models.len());
    for model in models {
        let name = model
            .get("name")
            .and_then(Value::as_str)
            .ok_or_else(|| OllamaError::protocol("Ollama returned a model without a name."))?;
        let name = normalize_model_name(name)?;
        if !normalized_names.insert(name.to_ascii_lowercase()) {
            return Err(OllamaError::protocol(format!(
                "Ollama returned duplicate model name `{name}`."
            )));
        }
        names.push(name);
    }
    Ok(names)
}

fn parse_show_model(name: &str, response: &Value) -> Result<OllamaModel, OllamaError> {
    let capabilities = response
        .get("capabilities")
        .and_then(Value::as_array)
        .ok_or_else(|| {
            OllamaError::protocol(format!(
                "Ollama returned no capability metadata for `{name}`."
            ))
        })?;
    let mut normalized = capabilities
        .iter()
        .map(|capability| {
            capability.as_str().ok_or_else(|| {
                OllamaError::protocol(format!(
                    "Ollama returned malformed capability metadata for `{name}`."
                ))
            })
        })
        .collect::<Result<Vec<_>, _>>()?
        .into_iter()
        .map(str::trim)
        .filter(|capability| !capability.is_empty())
        .map(str::to_ascii_lowercase)
        .collect::<Vec<_>>();
    normalized.sort();
    normalized.dedup();
    let context_length = response
        .get("model_info")
        .and_then(Value::as_object)
        .and_then(context_length_from_model_info);
    Ok(OllamaModel {
        name: normalize_model_name(name)?,
        capabilities: normalized,
        context_length,
        availability: ProviderAvailability::Ready,
        message: if context_length.is_some() {
            "Model capabilities and context metadata are ready.".to_string()
        } else {
            "Model capabilities are ready; context length is unavailable.".to_string()
        },
    })
}

fn context_length_from_model_info(model_info: &Map<String, Value>) -> Option<u64> {
    let architecture = model_info
        .get("general.architecture")
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|architecture| !architecture.is_empty());
    if let Some(architecture) = architecture {
        let key = format!("{architecture}.context_length");
        if let Some(context_length) = bounded_context_length(model_info.get(&key)) {
            return Some(context_length);
        }
    }
    let mut candidates = model_info
        .iter()
        .filter(|(key, _)| key.ends_with(".context_length"))
        .filter_map(|(_, value)| bounded_context_length(Some(value)))
        .collect::<Vec<_>>();
    candidates.sort_unstable();
    candidates.dedup();
    (candidates.len() == 1).then(|| candidates[0])
}

fn bounded_context_length(value: Option<&Value>) -> Option<u64> {
    value
        .and_then(Value::as_u64)
        .filter(|context| (1..=MAX_OLLAMA_CONTEXT_TOKENS).contains(context))
}

fn normalize_model_name(name: &str) -> Result<String, OllamaError> {
    let name = name.trim();
    if name.is_empty()
        || name.len() > MAX_OLLAMA_MODEL_NAME_BYTES
        || name.contains(['\0', '\r', '\n'])
    {
        return Err(OllamaError::protocol(
            "Ollama returned an invalid model name.",
        ));
    }
    Ok(name.to_string())
}

fn validate_loopback_endpoint(url: &Url) -> Result<(), OllamaError> {
    let loopback = url
        .host_str()
        .and_then(|host| host.parse::<IpAddr>().ok())
        .map(|address| address.is_loopback())
        .unwrap_or(false);
    if url.scheme() != "http"
        || !loopback
        || !url.username().is_empty()
        || url.password().is_some()
        || url.query().is_some()
        || url.fragment().is_some()
    {
        return Err(OllamaError::protocol(
            "Ollama requests are restricted to an unauthenticated numeric HTTP loopback endpoint.",
        ));
    }
    Ok(())
}

fn map_reqwest_error(error: reqwest::Error) -> OllamaError {
    if error.is_timeout() {
        OllamaError::timed_out()
    } else if error.is_connect() {
        OllamaError::unavailable(format!(
            "Ollama is not reachable at {OLLAMA_DISPLAY_ENDPOINT}: {error}"
        ))
    } else {
        OllamaError::protocol(format!("The Ollama HTTP exchange failed: {error}"))
    }
}

fn bounded_error_detail(detail: &str) -> String {
    detail.trim().chars().take(MAX_ERROR_DETAIL_CHARS).collect()
}

async fn wait_for_cancellation(cancellation: Option<ProviderCancellation>) {
    let Some(cancellation) = cancellation else {
        pending::<()>().await;
        return;
    };
    loop {
        if cancellation.is_cancelled() {
            return;
        }
        tokio::time::sleep(OLLAMA_CANCEL_POLL_INTERVAL).await;
    }
}

#[cfg(test)]
mod tests;
