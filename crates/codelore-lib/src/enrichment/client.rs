//! Synchronous two-dialect chat client for the advisory narrative layer.
//!
//! The advisory layer speaks to whatever model the operator points it at, in
//! one of two request shapes: the Anthropic-native `/v1/messages` API, or the
//! OpenAI-compatible `/chat/completions` API that local runners (Ollama, LM
//! Studio, vLLM) expose. The posture is local-first — with nothing configured,
//! resolution defaults to a local OpenAI-compatible endpoint.
//!
//! The client is deliberately blocking: `codelore-lib` carries no async
//! runtime, and one advisory completion per invocation does not warrant one. It
//! uses a single [`ureq`] agent with one total timeout and no retries.
//!
//! Environment reading is confined to [`LlmEnv::from_process_env`]; every other
//! path takes an [`LlmEnv`] by reference, so [`resolve_client`] is pure over its
//! input and the resolution matrix is unit-testable without environment races.

use std::time::Duration;

use serde_json::json;
use ureq::Agent;

use crate::{CodeLoreError, Result};

/// Model used for the Anthropic dialect when the operator sets no model.
pub const DEFAULT_ANTHROPIC_MODEL: &str = "claude-sonnet-5";

/// Base URL for the OpenAI-compatible dialect when the operator sets none —
/// Ollama's local OpenAI-compatible shim; this default makes the layer local-first.
pub const DEFAULT_OPENAI_COMPAT_BASE_URL: &str = "http://localhost:11434/v1";

/// Base URL for the Anthropic dialect when the operator sets none.
pub const DEFAULT_ANTHROPIC_BASE_URL: &str = "https://api.anthropic.com";

/// Total per-request timeout, in seconds. Covers connect through body read.
pub const REQUEST_TIMEOUT_SECS: u64 = 120;

/// Anthropic messages API version, pinned in the `anthropic-version` header.
const ANTHROPIC_VERSION: &str = "2023-06-01";

/// Response token ceiling for the advisory completion. A narrative is a few
/// tight paragraphs; this bounds a runaway generation.
const MAX_TOKENS: u32 = 1024;

/// A model that turns one system + user exchange into assistant text.
pub trait ChatClient {
    /// Complete a single system/user exchange, returning the assistant's text.
    fn complete(&self, system: &str, user: &str) -> Result<String>;
    /// The model identifier this client targets, for the advisory stamp.
    fn model_id(&self) -> &str;
}

/// Build the shared blocking agent: one global timeout, no retries, and
/// non-2xx responses surfaced as `Ok` so the response body can be folded into
/// the error message rather than discarded.
fn build_agent() -> Agent {
    Agent::config_builder()
        .timeout_global(Some(Duration::from_secs(REQUEST_TIMEOUT_SECS)))
        .http_status_as_error(false)
        .build()
        .into()
}

/// POST `body` as JSON to `url` with `headers`, returning the parsed response.
///
/// The agent disables status-as-error, so a non-2xx response arrives as `Ok`;
/// this maps it to [`CodeLoreError::Analysis`] carrying the status code and the
/// first 200 characters of the response body.
fn post_json(
    agent: &Agent,
    url: &str,
    headers: &[(&str, &str)],
    body: &serde_json::Value,
) -> Result<serde_json::Value> {
    let mut request = agent.post(url);
    for (name, value) in headers {
        request = request.header(*name, *value);
    }
    let mut response = request
        .send_json(body)
        .map_err(|e| CodeLoreError::Analysis(format!("LLM request to {url} failed: {e}")))?;
    let status = response.status();
    let text = response.body_mut().read_to_string().map_err(|e| {
        CodeLoreError::Analysis(format!("reading LLM response from {url} failed: {e}"))
    })?;
    if !status.is_success() {
        let snippet: String = text.chars().take(200).collect();
        return Err(CodeLoreError::Analysis(format!(
            "LLM request to {url} returned HTTP {}: {snippet}",
            status.as_u16()
        )));
    }
    serde_json::from_str(&text).map_err(|e| {
        CodeLoreError::Analysis(format!("LLM response from {url} was not valid JSON: {e}"))
    })
}

/// Client for the Anthropic-native `/v1/messages` API.
pub struct AnthropicClient {
    agent: Agent,
    api_key: String,
    model: String,
    base_url: String,
}

impl AnthropicClient {
    /// A client posting to `{base_url}/v1/messages` as `model`, authenticated
    /// with `api_key`.
    #[must_use]
    pub fn new(api_key: String, model: String, base_url: String) -> Self {
        Self {
            agent: build_agent(),
            api_key,
            model,
            base_url,
        }
    }
}

impl ChatClient for AnthropicClient {
    fn complete(&self, system: &str, user: &str) -> Result<String> {
        let url = format!("{}/v1/messages", self.base_url.trim_end_matches('/'));
        let body = json!({
            "model": self.model,
            "max_tokens": MAX_TOKENS,
            "system": system,
            "messages": [{ "role": "user", "content": user }],
        });
        let value = post_json(
            &self.agent,
            &url,
            &[
                ("x-api-key", self.api_key.as_str()),
                ("anthropic-version", ANTHROPIC_VERSION),
            ],
            &body,
        )?;
        value["content"][0]["text"]
            .as_str()
            .map(str::to_owned)
            .ok_or_else(|| {
                CodeLoreError::Analysis(format!(
                    "Anthropic response from {url} had no text at content[0].text"
                ))
            })
    }

    fn model_id(&self) -> &str {
        &self.model
    }
}

/// Client for the OpenAI-compatible `/chat/completions` API.
pub struct OpenAiCompatClient {
    agent: Agent,
    base_url: String,
    api_key: Option<String>,
    model: String,
}

impl OpenAiCompatClient {
    /// A client posting to `{base_url}/chat/completions` as `model`. When
    /// `api_key` is set it is sent as an `Authorization: Bearer` header; local
    /// runners typically need none.
    #[must_use]
    pub fn new(base_url: String, api_key: Option<String>, model: String) -> Self {
        Self {
            agent: build_agent(),
            base_url,
            api_key,
            model,
        }
    }
}

impl ChatClient for OpenAiCompatClient {
    fn complete(&self, system: &str, user: &str) -> Result<String> {
        let url = format!("{}/chat/completions", self.base_url.trim_end_matches('/'));
        let body = json!({
            "model": self.model,
            "messages": [
                { "role": "system", "content": system },
                { "role": "user", "content": user },
            ],
        });
        let authorization = self.api_key.as_ref().map(|key| format!("Bearer {key}"));
        let mut headers: Vec<(&str, &str)> = Vec::new();
        if let Some(value) = &authorization {
            headers.push(("authorization", value.as_str()));
        }
        let value = post_json(&self.agent, &url, &headers, &body)?;
        value["choices"][0]["message"]["content"]
            .as_str()
            .map(str::to_owned)
            .ok_or_else(|| {
                CodeLoreError::Analysis(format!(
                    "OpenAI-compatible response from {url} had no text at choices[0].message.content"
                ))
            })
    }

    fn model_id(&self) -> &str {
        &self.model
    }
}

/// Raw environment inputs for client resolution, read once at the crate's only
/// environment-touching site so that resolution itself stays pure.
///
/// Deliberately does NOT derive `Debug`: two fields carry API keys, and a
/// derived impl would print them through any `{:?}` or `tracing` sink.
#[derive(Clone, Default)]
pub struct LlmEnv {
    /// `CODELORE_LLM_PROVIDER` — `anthropic` or `openai-compat`; unset lets the
    /// presence of an Anthropic key (else the local default) decide.
    pub provider: Option<String>,
    /// `ANTHROPIC_API_KEY` — the Anthropic dialect's credential.
    pub anthropic_key: Option<String>,
    /// `CODELORE_LLM_BASE_URL` — the OpenAI-compatible endpoint base.
    pub base_url: Option<String>,
    /// `CODELORE_LLM_API_KEY` — optional bearer token for the OpenAI-compatible
    /// endpoint.
    pub api_key: Option<String>,
    /// `CODELORE_LLM_MODEL` — required for the OpenAI-compatible dialect;
    /// overrides the default Anthropic model when the Anthropic dialect is used.
    pub model: Option<String>,
}

impl LlmEnv {
    /// Read the LLM configuration from the process environment. Empty or
    /// whitespace-only values are treated as unset. This is the crate's only
    /// environment-reading site.
    #[must_use]
    pub fn from_process_env() -> Self {
        Self {
            provider: read_env("CODELORE_LLM_PROVIDER"),
            anthropic_key: read_env("ANTHROPIC_API_KEY"),
            base_url: read_env("CODELORE_LLM_BASE_URL"),
            api_key: read_env("CODELORE_LLM_API_KEY"),
            model: read_env("CODELORE_LLM_MODEL"),
        }
    }
}

/// Read an environment variable, treating empty/whitespace-only values as unset.
fn read_env(name: &str) -> Option<String> {
    std::env::var(name)
        .ok()
        .map(|value| value.trim().to_string())
        .filter(|value| !value.is_empty())
}

/// The concrete client an [`LlmEnv`] selects, before the (infallible) HTTP
/// agent is built. Split out from [`resolve_client`] so the resolution matrix
/// is unit-testable without constructing an agent.
///
/// `Debug` is implemented by hand (not derived) for the same reason [`LlmEnv`]
/// omits it: both variants carry credential material, and a derived impl would
/// print the key through any `{:?}` or `tracing` sink. The manual impl redacts
/// the key while preserving the present/absent distinction for the optional
/// bearer token.
enum Resolved {
    Anthropic {
        api_key: String,
        model: String,
        base_url: String,
    },
    OpenAiCompat {
        base_url: String,
        api_key: Option<String>,
        model: String,
    },
}

impl std::fmt::Debug for Resolved {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Anthropic {
                model, base_url, ..
            } => f
                .debug_struct("Anthropic")
                .field("api_key", &"<redacted>")
                .field("model", model)
                .field("base_url", base_url)
                .finish(),
            Self::OpenAiCompat {
                base_url,
                api_key,
                model,
            } => f
                .debug_struct("OpenAiCompat")
                .field("base_url", base_url)
                // Preserve whether a bearer token was supplied without
                // printing it: `Some("<redacted>")` vs `None`.
                .field("api_key", &api_key.as_ref().map(|_| "<redacted>"))
                .field("model", model)
                .finish(),
        }
    }
}

/// The valid `CODELORE_LLM_PROVIDER` values, echoed in the unknown-provider
/// error.
const VALID_PROVIDERS: &str = "\"anthropic\" or \"openai-compat\"";

/// Choose a client shape from environment inputs. Explicit provider wins; with
/// none, an Anthropic key implies the Anthropic dialect, otherwise the
/// local-first OpenAI-compatible dialect.
fn resolve(env: &LlmEnv) -> Result<Resolved> {
    match env.provider.as_deref().map(str::trim) {
        Some(provider) if provider.eq_ignore_ascii_case("anthropic") => resolve_anthropic(env),
        Some(provider) if provider.eq_ignore_ascii_case("openai-compat") => {
            resolve_openai_compat(env)
        }
        Some(other) => Err(CodeLoreError::Analysis(format!(
            "unknown CODELORE_LLM_PROVIDER {other:?} — set it to {VALID_PROVIDERS}"
        ))),
        None if env.anthropic_key.is_some() => resolve_anthropic(env),
        None => resolve_openai_compat(env),
    }
}

/// Resolve the Anthropic dialect. Requires an Anthropic key; the base URL is
/// always the Anthropic default (the local base env var is OpenAI-shaped).
fn resolve_anthropic(env: &LlmEnv) -> Result<Resolved> {
    let api_key = env.anthropic_key.clone().ok_or_else(|| {
        CodeLoreError::Analysis(
            "Anthropic provider selected but ANTHROPIC_API_KEY is not set".to_string(),
        )
    })?;
    Ok(Resolved::Anthropic {
        api_key,
        model: env
            .model
            .clone()
            .unwrap_or_else(|| DEFAULT_ANTHROPIC_MODEL.to_string()),
        base_url: DEFAULT_ANTHROPIC_BASE_URL.to_string(),
    })
}

/// Resolve the OpenAI-compatible dialect. Requires a model; the base URL falls
/// back to the local-first default.
fn resolve_openai_compat(env: &LlmEnv) -> Result<Resolved> {
    let model = env.model.clone().ok_or_else(|| {
        CodeLoreError::Analysis(
            "OpenAI-compatible provider requires a model — set CODELORE_LLM_MODEL \
             (e.g. from `ollama list`)"
                .to_string(),
        )
    })?;
    Ok(Resolved::OpenAiCompat {
        base_url: env
            .base_url
            .clone()
            .unwrap_or_else(|| DEFAULT_OPENAI_COMPAT_BASE_URL.to_string()),
        api_key: env.api_key.clone(),
        model,
    })
}

/// Resolve a [`ChatClient`] from environment inputs (local-first).
///
/// With an explicit `CODELORE_LLM_PROVIDER`, that dialect is used — the
/// Anthropic dialect requires `ANTHROPIC_API_KEY`, the OpenAI-compatible dialect
/// requires `CODELORE_LLM_MODEL`. With no provider set, an `ANTHROPIC_API_KEY`
/// selects the Anthropic dialect; otherwise resolution falls to the local-first
/// OpenAI-compatible dialect (which still requires a model).
pub fn resolve_client(env: &LlmEnv) -> Result<Box<dyn ChatClient>> {
    Ok(match resolve(env)? {
        Resolved::Anthropic {
            api_key,
            model,
            base_url,
        } => Box::new(AnthropicClient::new(api_key, model, base_url)),
        Resolved::OpenAiCompat {
            base_url,
            api_key,
            model,
        } => Box::new(OpenAiCompatClient::new(base_url, api_key, model)),
    })
}

#[cfg(test)]
mod tests {
    use super::{
        DEFAULT_ANTHROPIC_BASE_URL, DEFAULT_ANTHROPIC_MODEL, DEFAULT_OPENAI_COMPAT_BASE_URL,
        LlmEnv, Resolved, resolve, resolve_client,
    };

    #[test]
    fn explicit_anthropic_resolves_with_default_model_and_base() {
        let env = LlmEnv {
            provider: Some("anthropic".to_string()),
            anthropic_key: Some("secret".to_string()),
            ..LlmEnv::default()
        };
        match resolve(&env).expect("resolves") {
            Resolved::Anthropic {
                api_key,
                model,
                base_url,
            } => {
                assert_eq!(api_key, "secret");
                assert_eq!(model, DEFAULT_ANTHROPIC_MODEL);
                assert_eq!(base_url, DEFAULT_ANTHROPIC_BASE_URL);
            }
            Resolved::OpenAiCompat { .. } => panic!("expected the Anthropic dialect"),
        }
    }

    #[test]
    fn explicit_anthropic_without_key_names_the_key_var() {
        let env = LlmEnv {
            provider: Some("anthropic".to_string()),
            ..LlmEnv::default()
        };
        let err = resolve(&env).expect_err("missing key must error");
        assert!(
            err.to_string().contains("ANTHROPIC_API_KEY"),
            "error should name the key var: {err}"
        );
    }

    #[test]
    fn explicit_openai_compat_resolves_with_default_base() {
        let env = LlmEnv {
            provider: Some("openai-compat".to_string()),
            model: Some("llama3".to_string()),
            ..LlmEnv::default()
        };
        match resolve(&env).expect("resolves") {
            Resolved::OpenAiCompat {
                base_url,
                api_key,
                model,
            } => {
                assert_eq!(base_url, DEFAULT_OPENAI_COMPAT_BASE_URL);
                assert_eq!(api_key, None);
                assert_eq!(model, "llama3");
            }
            Resolved::Anthropic { .. } => panic!("expected the OpenAI-compatible dialect"),
        }
    }

    #[test]
    fn explicit_openai_compat_without_model_names_the_model_var_and_ollama() {
        let env = LlmEnv {
            provider: Some("openai-compat".to_string()),
            ..LlmEnv::default()
        };
        let err = resolve(&env).expect_err("missing model must error");
        let message = err.to_string();
        assert!(
            message.contains("CODELORE_LLM_MODEL"),
            "error should name the model var: {message}"
        );
        assert!(
            message.contains("ollama list"),
            "error should suggest `ollama list`: {message}"
        );
    }

    #[test]
    fn an_anthropic_key_implies_the_anthropic_dialect() {
        let env = LlmEnv {
            anthropic_key: Some("secret".to_string()),
            ..LlmEnv::default()
        };
        assert!(matches!(
            resolve(&env).expect("resolves"),
            Resolved::Anthropic { .. }
        ));
    }

    #[test]
    fn no_config_but_a_model_resolves_to_the_local_default() {
        let env = LlmEnv {
            model: Some("llama3".to_string()),
            ..LlmEnv::default()
        };
        match resolve(&env).expect("resolves") {
            Resolved::OpenAiCompat { base_url, .. } => {
                assert_eq!(base_url, DEFAULT_OPENAI_COMPAT_BASE_URL);
            }
            Resolved::Anthropic { .. } => panic!("expected the local-first dialect"),
        }
    }

    #[test]
    fn debug_redacts_credential_material() {
        // Anthropic dialect: the required key must never appear in `{:?}`.
        let anthropic = resolve(&LlmEnv {
            anthropic_key: Some("sk-ant-super-secret".to_string()),
            ..LlmEnv::default()
        })
        .expect("resolves");
        let rendered = format!("{anthropic:?}");
        assert!(
            !rendered.contains("sk-ant-super-secret"),
            "Debug leaked the Anthropic key: {rendered}"
        );
        assert!(
            rendered.contains("<redacted>"),
            "Debug should mark the key redacted: {rendered}"
        );

        // OpenAI-compatible dialect: the optional bearer token is redacted but
        // its presence stays visible (Some vs None).
        let openai = resolve(&LlmEnv {
            provider: Some("openai-compat".to_string()),
            base_url: Some("http://localhost:1234/v1".to_string()),
            api_key: Some("bearer-token-do-not-log".to_string()),
            model: Some("llama3".to_string()),
            ..LlmEnv::default()
        })
        .expect("resolves");
        let rendered = format!("{openai:?}");
        assert!(
            !rendered.contains("bearer-token-do-not-log"),
            "Debug leaked the bearer token: {rendered}"
        );
        assert!(
            rendered.contains("Some(\"<redacted>\")"),
            "Debug should show the token present-but-redacted: {rendered}"
        );
    }

    #[test]
    fn unknown_provider_names_the_valid_values() {
        let env = LlmEnv {
            provider: Some("gpt4all".to_string()),
            ..LlmEnv::default()
        };
        let err = resolve(&env).expect_err("unknown provider must error");
        let message = err.to_string();
        assert!(
            message.contains("anthropic") && message.contains("openai-compat"),
            "error should list the valid providers: {message}"
        );
    }

    #[test]
    fn resolve_client_wires_the_model_id() {
        let anthropic = LlmEnv {
            anthropic_key: Some("secret".to_string()),
            ..LlmEnv::default()
        };
        assert_eq!(
            resolve_client(&anthropic).expect("client").model_id(),
            DEFAULT_ANTHROPIC_MODEL
        );

        let local = LlmEnv {
            model: Some("llama3".to_string()),
            ..LlmEnv::default()
        };
        assert_eq!(resolve_client(&local).expect("client").model_id(), "llama3");
    }
}
