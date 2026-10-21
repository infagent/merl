//! Provider-neutral, fixed-endpoint HTTP transport outside network-denied workers.
//!
//! Provider translation is a pinned, network-denied JSON process. Only this broker
//! owns credentials and HTTP. It accepts text generation requests, never worker-
//! supplied destinations, headers, filesystem paths, or executable commands.

use crate::isolation::{
    IsolatedProgram, ProcessCallRecord, ProcessLimits, hex_digest, read_bounded,
};
use crate::{BenchmarkConfig, RandomnessControl, TokenUsage};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use std::{collections::BTreeMap, error::Error, fmt, io::Read, path::PathBuf, time::Duration};

// Control-protocol ceilings are independent of token accounting. Sixteen
// explicit attempts allow correction loops while bounding potential billing.
const MAX_CALLS: usize = 16;
const MAX_MESSAGES: usize = 128;
const MAX_CONFIG_BYTES: usize = 1_048_576;
const MAX_CREDENTIAL_HEADERS: usize = 4;
const MAX_HEADER_VALUE_BYTES: usize = 8_192;
const MAX_HTTP_HEADER_BYTES: usize = 32_768;
const MAX_IDENTIFIER_BYTES: usize = 256;
const MAX_ERROR_CODE_BYTES: usize = 64;

/// The controller chooses a role; workers cannot change it in a model-call frame.
#[derive(Clone, Copy, Debug, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum TransportRole {
    /// Answering and rolling-summary generation from disclosed context.
    Reader,
    /// Candidate compilation from an explicitly prepared prefix.
    Preparer,
    /// Evaluator-owned semantic grading, with its separate configuration.
    Grader,
}

/// A configuration file pinned by its exact bytes, without credential values.
#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct TransportPin {
    /// Evaluator-owned configuration file, never mounted in a worker.
    pub path: PathBuf,
    /// SHA-256 of the configuration bytes, optionally prefixed by `sha256:`.
    pub sha256: String,
}

/// Provider controls that a concrete adapter must implement or explicitly reject.
#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct ModelSettings {
    /// Configured provider model identifier.
    pub model: String,
    /// Expected version, or an explicit provider limitation.
    pub model_version: String,
    /// Reasoning control, including explicit `none`.
    pub effort: String,
    /// Sampling temperature when supported.
    pub temperature: Option<f64>,
    /// Provider-side output ceiling, enforced by the concrete adapter.
    pub max_output_tokens: u64,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct CredentialHeader {
    name: String,
    env: String,
    prefix: String,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct Config {
    schema: String,
    role: TransportRole,
    endpoint: String,
    credential_headers: Vec<CredentialHeader>,
    adapter_program: PathBuf,
    adapter_program_sha256: String,
    settings: ModelSettings,
    limits: ProcessLimits,
    max_calls: usize,
}

/// Non-sensitive receipt for one attempted generation, including failed calls.
#[derive(Clone, Debug, Serialize)]
pub struct RemoteCallRecord {
    /// Effective provider controls, including a compiler run's tighter output cap.
    pub settings: ModelSettings,
    /// Host-selected paired-trial randomness, never supplied by the worker.
    pub randomness: Option<RandomnessControl>,
    /// Host-issued worker invocation identity, not a worker-selected unit.
    pub worker_request_sha256: String,
    /// Unit and trial identity, when present in the host request.
    pub unit_id: Option<String>,
    /// Paired trial identity for reader/preparer calls.
    pub trial_id: Option<String>,
    /// Normalized request hash, without storing prompts.
    pub request_sha256: String,
    /// Exact JSON HTTP body submitted to the fixed endpoint.
    pub http_request_sha256: Option<String>,
    /// Exact HTTP response body, before provider translation.
    pub http_response_sha256: Option<String>,
    /// Status received from the pinned endpoint, including provider failures.
    pub http_status: Option<u16>,
    /// Whether a request may have reached the provider; unknown costs stay explicit.
    pub attempted: bool,
    /// Actual provider-reported usage; absent usage is never replaced with zero.
    pub usage: Option<TokenUsage>,
    /// Resolved provider model identifier, when supplied by the provider.
    pub reported_model: Option<String>,
    /// Provider correlation identity, when supplied.
    pub request_id: Option<String>,
    /// Stable broker or normalized provider failure class, without response prose.
    pub error: Option<String>,
    /// Exact encode/decode process hashes for the pinned provider adapter.
    pub adapter_calls: Vec<ProcessCallRecord>,
}

/// Transport provenance kept separate for reader, preparer and grader accounting.
#[derive(Debug, Serialize)]
pub struct TransportReport<'a> {
    /// Role bound at construction.
    pub role: TransportRole,
    /// Frozen non-secret configuration bytes.
    pub config_sha256: &'a str,
    /// Frozen provider translation executable.
    pub adapter_sha256: &'a str,
    /// Generation attempts, never collapsed into successful scoring units.
    pub calls: &'a [RemoteCallRecord],
}

/// A trusted fixed-endpoint broker; secrets never enter its debug output.
pub struct RemoteTransport {
    config: Config,
    config_sha256: String,
    adapter: IsolatedProgram,
    headers: Vec<(String, String)>,
    secrets: Vec<String>,
    agent: ureq::Agent,
    calls: Vec<RemoteCallRecord>,
    invocation_start: usize,
    worker_request: Value,
}
impl fmt::Debug for RemoteTransport {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("RemoteTransport")
            .field("role", &self.config.role)
            .field("config_sha256", &self.config_sha256)
            .field("adapter", &self.adapter)
            .field("call_count", &self.calls.len())
            .finish_non_exhaustive()
    }
}

impl RemoteTransport {
    /// Pins configuration and translation code, resolving only named credentials.
    ///
    /// Credentials come from the trusted controller's environment. Neither the
    /// configuration file nor these values are mounted into any child process.
    ///
    /// # Errors
    /// Rejects pin/role mismatches, invalid endpoints, missing credentials and bounds.
    pub fn open(pin: &TransportPin, role: TransportRole) -> Result<Self, TransportError> {
        Self::open_with_credentials(pin, role, |name| std::env::var(name).ok())
    }

    /// Resolves credentials through an explicit capability, useful for embedding.
    ///
    /// # Errors
    /// Returns the same configuration errors as [`Self::open`].
    pub fn open_with_credentials(
        pin: &TransportPin,
        role: TransportRole,
        mut credential: impl FnMut(&str) -> Option<String>,
    ) -> Result<Self, TransportError> {
        let bytes =
            read_bounded(&pin.path, MAX_CONFIG_BYTES).map_err(|_| TransportError::Configuration)?;
        let config_sha256 = hex_digest(&bytes);
        if !same_digest(&config_sha256, &pin.sha256) {
            return Err(TransportError::Pin);
        }
        let config: Config =
            serde_json::from_slice(&bytes).map_err(|_| TransportError::Configuration)?;
        if config.schema != "merl.remote-transport/v1"
            || config.role != role
            || !(1..=MAX_CALLS).contains(&config.max_calls)
            || config.settings.model.trim().is_empty()
            || config.settings.model_version.trim().is_empty()
            || config.settings.effort.trim().is_empty()
            || config.settings.max_output_tokens == 0
            || config
                .settings
                .temperature
                .is_some_and(|t| !t.is_finite() || t < 0.0)
            || config.credential_headers.len() > MAX_CREDENTIAL_HEADERS
        {
            return Err(TransportError::Configuration);
        }
        validate_endpoint(&config.endpoint)?;
        let adapter = IsolatedProgram::new(&config.adapter_program, config.limits)
            .map_err(|_| TransportError::Configuration)?;
        if !same_digest(adapter.sha256(), &config.adapter_program_sha256) {
            return Err(TransportError::Pin);
        }
        let mut headers = Vec::new();
        let mut secrets = Vec::new();
        for header in &config.credential_headers {
            let name = header.name.to_ascii_lowercase();
            if name.is_empty()
                || name.len() > 128
                || !name
                    .bytes()
                    .all(|b| b.is_ascii_alphanumeric() || matches!(b, b'-' | b'_'))
                || matches!(
                    name.as_str(),
                    "host"
                        | "content-length"
                        | "content-type"
                        | "transfer-encoding"
                        | "connection"
                        | "proxy-authorization"
                        | "accept-encoding"
                )
                || headers.iter().any(|(existing, _)| existing == &name)
                || header.env.is_empty()
                || header.env.len() > 128
                || !header
                    .env
                    .bytes()
                    .all(|b| b.is_ascii_alphanumeric() || b == b'_')
            {
                return Err(TransportError::Configuration);
            }
            let secret = credential(&header.env)
                .filter(|s| !s.is_empty())
                .ok_or(TransportError::Credentials)?;
            let value = format!("{}{secret}", header.prefix);
            if value.len() > MAX_HEADER_VALUE_BYTES
                || !value.bytes().all(|b| (32..=126).contains(&b))
            {
                return Err(TransportError::Credentials);
            }
            secrets.push(secret);
            headers.push((name, value));
        }
        let agent: ureq::Agent = ureq::Agent::config_builder()
            .proxy(None)
            .max_redirects(0)
            .http_status_as_error(false)
            .timeout_global(Some(Duration::from_secs(u64::from(config.limits.seconds))))
            .max_response_header_size(MAX_HTTP_HEADER_BYTES)
            .build()
            .into();
        Ok(Self {
            config,
            config_sha256,
            adapter,
            headers,
            secrets,
            agent,
            calls: Vec::new(),
            invocation_start: 0,
            worker_request: Value::Null,
        })
    }

    /// Checks reader configuration agrees with the shared benchmark controls.
    ///
    /// # Errors
    /// Rejects a role or model/version/effort/temperature mismatch before execution.
    pub fn check_reader(&self, config: &BenchmarkConfig) -> Result<(), TransportError> {
        let settings = &self.config.settings;
        if self.config.role != TransportRole::Reader
            || settings.model != config.model
            || settings.model_version != config.model_version
            || settings.effort != config.effort
            || settings.temperature != config.temperature
        {
            return Err(TransportError::Configuration);
        }
        Ok(())
    }

    /// Non-sensitive execution ledger, including failed and incomplete calls.
    #[must_use]
    pub fn report(&self) -> TransportReport<'_> {
        TransportReport {
            role: self.config.role,
            config_sha256: &self.config_sha256,
            adapter_sha256: self.adapter.sha256(),
            calls: &self.calls,
        }
    }

    /// Rejects use of a transport belonging to another execution role.
    ///
    /// # Errors
    /// Returns a configuration error when reader/preparer/grader capabilities differ.
    pub fn require_role(&self, role: TransportRole) -> Result<(), TransportError> {
        if self.config.role == role {
            Ok(())
        } else {
            Err(TransportError::Configuration)
        }
    }

    /// Known provider usage for the current worker invocation, without estimating gaps.
    ///
    /// # Errors
    /// Rejects arithmetic overflow instead of wrapping billed costs.
    pub fn invocation_usage(&self) -> Result<TokenUsage, TransportError> {
        self.calls[self.invocation_start..]
            .iter()
            .filter_map(|c| c.usage)
            .try_fold(TokenUsage::default(), |sum, usage| {
                Ok(TokenUsage {
                    input: sum
                        .input
                        .checked_add(usage.input)
                        .ok_or(TransportError::Usage)?,
                    output: sum
                        .output
                        .checked_add(usage.output)
                        .ok_or(TransportError::Usage)?,
                })
            })
    }

    pub(crate) fn begin(&mut self, request: &Value) {
        self.invocation_start = self.calls.len();
        self.worker_request = request.clone();
    }

    pub(crate) fn invocation_succeeded(&self) -> bool {
        self.calls.len() > self.invocation_start
            && self.calls[self.invocation_start..]
                .iter()
                .all(|c| c.error.is_none() && c.usage.is_some())
    }

    pub(crate) fn generate(&mut self, frame: Value) -> Result<Value, TransportError> {
        let mut record = RemoteCallRecord {
            settings: self.config.settings.clone(),
            randomness: None,
            worker_request_sha256: hex_digest(
                &serde_json::to_vec(&self.worker_request).map_err(|_| TransportError::Protocol)?,
            ),
            unit_id: self.worker_request["unit_id"]
                .as_str()
                .or_else(|| self.worker_request["question_id"].as_str())
                .map(ToOwned::to_owned),
            trial_id: self.worker_request["trial"]["id"]
                .as_str()
                .map(ToOwned::to_owned),
            request_sha256: hex_digest(
                &serde_json::to_vec(&frame).map_err(|_| TransportError::Protocol)?,
            ),
            http_request_sha256: None,
            http_response_sha256: None,
            http_status: None,
            attempted: false,
            usage: None,
            reported_model: None,
            request_id: None,
            error: None,
            adapter_calls: Vec::new(),
        };
        let result = if self.calls.len() - self.invocation_start >= self.config.max_calls {
            Err(TransportError::CallLimit)
        } else {
            self.perform(frame, &mut record)
        };
        if let Err(error) = &result
            && record.error.is_none()
        {
            record.error = Some(error.code().into());
        }
        self.calls.push(record);
        result
    }

    fn perform(
        &self,
        frame: Value,
        record: &mut RemoteCallRecord,
    ) -> Result<Value, TransportError> {
        #[derive(Deserialize)]
        #[serde(deny_unknown_fields)]
        struct Call {
            schema: String,
            messages: Vec<Message>,
        }
        #[derive(Deserialize, Serialize)]
        #[serde(deny_unknown_fields)]
        struct Message {
            role: MessageRole,
            text: String,
        }
        #[derive(Deserialize, Serialize)]
        #[serde(rename_all = "snake_case")]
        enum MessageRole {
            System,
            User,
            Assistant,
        }
        let call: Call = serde_json::from_value(frame).map_err(|_| TransportError::Protocol)?;
        if call.schema != "merl.model-call/v1"
            || call.messages.is_empty()
            || call.messages.len() > MAX_MESSAGES
        {
            return Err(TransportError::Protocol);
        }
        let randomness = self.worker_request["trial"]
            .get("randomness")
            .map(|v| {
                serde_json::from_value::<RandomnessControl>(v.clone())
                    .map_err(|_| TransportError::Protocol)
            })
            .transpose()?;
        let mut settings = self.config.settings.clone();
        if self.config.role == TransportRole::Preparer {
            if self.worker_request["schema"] != "merl.compiler-request/v1" {
                return Err(TransportError::Protocol);
            }
            let cap = self.worker_request["limits"]["output_tokens"]
                .as_u64()
                .filter(|cap| *cap > 0)
                .ok_or(TransportError::Protocol)?;
            settings.max_output_tokens = settings.max_output_tokens.min(cap);
        }
        record.settings = settings.clone();
        record.randomness.clone_from(&randomness);
        let encode = json!({"schema":"merl.provider-adapter-request/v1","kind":"encode",
            "settings":settings,"randomness":randomness,"messages":call.messages});
        let encoded = self
            .adapter
            .recorded_call(&BTreeMap::new(), &encode, &mut record.adapter_calls)
            .map_err(|_| TransportError::Adapter)?;
        let ProviderResponse::Encoded { schema, body } =
            serde_json::from_value(encoded).map_err(|_| TransportError::Adapter)?
        else {
            return Err(TransportError::Adapter);
        };
        if schema != "merl.provider-adapter-response/v1" || !body.is_object() {
            return Err(TransportError::Adapter);
        }
        let bytes = serde_json::to_vec(&body).map_err(|_| TransportError::Adapter)?;
        if bytes.len() > self.config.limits.request_bytes {
            return Err(TransportError::Limit);
        }
        record.http_request_sha256 = Some(hex_digest(&bytes));
        let (status, body) = self.post(&bytes, record)?;
        let decode = json!({"schema":"merl.provider-adapter-request/v1","kind":"decode","status":status,"body":body});
        let decoded = self
            .adapter
            .recorded_call(&BTreeMap::new(), &decode, &mut record.adapter_calls)
            .map_err(|_| TransportError::Adapter)?;
        let decoded: ProviderResponse =
            serde_json::from_value(decoded).map_err(|_| TransportError::Adapter)?;
        decode_generation(decoded, status, record)
    }

    fn post(
        &self,
        bytes: &[u8],
        record: &mut RemoteCallRecord,
    ) -> Result<(u16, Value), TransportError> {
        let mut request = self
            .agent
            .post(&self.config.endpoint)
            .header("Content-Type", "application/json")
            .header("Accept", "application/json");
        for (name, value) in &self.headers {
            let mut value = ureq::http::HeaderValue::from_str(value)
                .map_err(|_| TransportError::Credentials)?;
            value.set_sensitive(true);
            request = request.header(name, value);
        }
        record.attempted = true;
        let mut response = request.send(bytes).map_err(|_| TransportError::Http)?;
        let status = response.status().as_u16();
        record.http_status = Some(status);
        let mut response_bytes = Vec::new();
        response
            .body_mut()
            .as_reader()
            .take((self.config.limits.response_bytes as u64).saturating_add(1))
            .read_to_end(&mut response_bytes)
            .map_err(|_| TransportError::Http)?;
        if response_bytes.len() > self.config.limits.response_bytes {
            return Err(TransportError::Limit);
        }
        record.http_response_sha256 = Some(hex_digest(&response_bytes));
        // The broker never follows redirects, including to other loopback services.
        if (300..400).contains(&status) {
            return Err(TransportError::Redirect);
        }
        if self
            .secrets
            .iter()
            .any(|s| response_bytes.windows(s.len()).any(|w| w == s.as_bytes()))
        {
            return Err(TransportError::CredentialEcho);
        }
        let body: Value =
            serde_json::from_slice(&response_bytes).map_err(|_| TransportError::Protocol)?;
        // JSON escapes can hide credentials on the wire. Check decoded keys and
        // strings before the provider translator can receive the response.
        if self
            .secrets
            .iter()
            .any(|secret| contains_credential(&body, secret))
        {
            return Err(TransportError::CredentialEcho);
        }
        Ok((status, body))
    }
}

fn contains_credential(value: &Value, secret: &str) -> bool {
    match value {
        Value::String(text) => text.contains(secret),
        Value::Array(values) => values
            .iter()
            .any(|value| contains_credential(value, secret)),
        Value::Object(fields) => fields
            .iter()
            .any(|(key, value)| key.contains(secret) || contains_credential(value, secret)),
        _ => false,
    }
}

fn decode_generation(
    decoded: ProviderResponse,
    status: u16,
    record: &mut RemoteCallRecord,
) -> Result<Value, TransportError> {
    let (schema, usage, model, request_id, text, error) = match decoded {
        ProviderResponse::Success {
            schema,
            text,
            usage,
            model,
            request_id,
        } => (schema, Some(usage), model, request_id, Some(text), None),
        ProviderResponse::Failure {
            schema,
            code,
            usage,
            model,
            request_id,
        } => (schema, usage, model, request_id, None, Some(code)),
        ProviderResponse::Encoded { .. } => return Err(TransportError::Adapter),
    };
    record.usage = usage;
    // Provider identifiers are opaque, bounded text. Keep prose out of error codes.
    if model.as_ref().is_some_and(|v| !valid_identifier(v))
        || request_id.as_ref().is_some_and(|v| !valid_identifier(v))
    {
        return Err(TransportError::Protocol);
    }
    record.reported_model = model;
    record.request_id = request_id;
    if schema != "merl.provider-adapter-response/v1" {
        return Err(TransportError::Adapter);
    }
    if let Some(code) = error {
        record.error = Some(if valid_code(&code) {
            code
        } else {
            "provider_failure".into()
        });
        return Err(TransportError::Provider);
    }
    if !(200..300).contains(&status) {
        return Err(TransportError::Http);
    }
    Ok(
        json!({"schema":"merl.model-result/v1","kind":"success","text":text,
            "usage":usage,"model":record.reported_model,"request_id":record.request_id}),
    )
}

#[derive(Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
enum ProviderResponse {
    Encoded {
        schema: String,
        body: Value,
    },
    Success {
        schema: String,
        text: String,
        usage: TokenUsage,
        model: Option<String>,
        request_id: Option<String>,
    },
    Failure {
        schema: String,
        code: String,
        usage: Option<TokenUsage>,
        model: Option<String>,
        request_id: Option<String>,
    },
}

fn valid_identifier(value: &str) -> bool {
    !value.is_empty() && value.len() <= MAX_IDENTIFIER_BYTES && !value.chars().any(char::is_control)
}
fn valid_code(value: &str) -> bool {
    !value.is_empty()
        && value.len() <= MAX_ERROR_CODE_BYTES
        && value.bytes().all(|b| b.is_ascii_lowercase() || b == b'_')
}
fn same_digest(actual: &str, pin: &str) -> bool {
    actual == pin.strip_prefix("sha256:").unwrap_or(pin)
}

fn validate_endpoint(endpoint: &str) -> Result<(), TransportError> {
    let uri: ureq::http::Uri = endpoint
        .parse()
        .map_err(|_| TransportError::Configuration)?;
    let host = uri.host().ok_or(TransportError::Configuration)?;
    let authority = uri.authority().ok_or(TransportError::Configuration)?;
    let loopback = host
        .trim_matches(['[', ']'])
        .parse::<std::net::IpAddr>()
        .is_ok_and(|a| a.is_loopback());
    if endpoint.len() > 2_048
        || endpoint.contains(['#', '\\', '@'])
        || uri.query().is_some()
        || authority.as_str().contains('@')
        || endpoint.bytes().any(|b| b <= 32 || b >= 127)
        || !(uri.scheme_str() == Some("https") || uri.scheme_str() == Some("http") && loopback)
    {
        return Err(TransportError::Configuration);
    }
    Ok(())
}

/// Safe failure categories; provider bodies, endpoints and credentials are excluded.
#[derive(Clone, Copy, Debug)]
pub enum TransportError {
    /// Invalid or unsupported transport configuration.
    Configuration,
    /// Configuration or adapter content differs from its pin.
    Pin,
    /// A configured credential is missing or invalid.
    Credentials,
    /// Worker or provider JSON violates the protocol.
    Protocol,
    /// A pinned provider translator failed.
    Adapter,
    /// The endpoint could not complete a bounded HTTP exchange.
    Http,
    /// The endpoint tried to redirect the request.
    Redirect,
    /// Provider output echoed a credential; it was withheld from children.
    CredentialEcho,
    /// A provider rejected generation, possibly with reported usage.
    Provider,
    /// A configured byte ceiling was exceeded.
    Limit,
    /// The worker exhausted its generation-call allowance.
    CallLimit,
    /// Reported usage overflowed the aggregate representation.
    Usage,
}
impl TransportError {
    /// Stable non-sensitive receipt code.
    #[must_use]
    pub fn code(self) -> &'static str {
        match self {
            Self::Configuration => "transport_configuration",
            Self::Pin => "transport_pin",
            Self::Credentials => "transport_credentials",
            Self::Protocol => "transport_protocol",
            Self::Adapter => "provider_adapter",
            Self::Http => "provider_http",
            Self::Redirect => "provider_redirect",
            Self::CredentialEcho => "credential_echo",
            Self::Provider => "provider_failure",
            Self::Limit => "transport_limit",
            Self::CallLimit => "model_call_limit",
            Self::Usage => "usage_overflow",
        }
    }
}
impl fmt::Display for TransportError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.code())
    }
}
impl Error for TransportError {}
