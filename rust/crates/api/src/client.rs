use crate::error::ApiError;
use crate::prompt_cache::{PromptCache, PromptCacheRecord, PromptCacheStats};
use crate::providers::anthropic::{self, AnthropicClient, AuthSource};
use crate::providers::openai_compat::{self, OpenAiCompatClient, OpenAiCompatConfig};
use crate::providers::{self, Provider, ProviderKind};
use crate::types::{MessageRequest, MessageResponse, StreamEvent};

async fn send_via_provider<P: Provider>(
    provider: &P,
    request: &MessageRequest,
) -> Result<MessageResponse, ApiError> {
    provider.send_message(request).await
}

async fn stream_via_provider<P: Provider>(
    provider: &P,
    request: &MessageRequest,
) -> Result<P::Stream, ApiError> {
    provider.stream_message(request).await
}

#[allow(clippy::large_enum_variant)]
#[derive(Debug, Clone)]
pub enum ProviderClient {
    Anthropic(AnthropicClient),
    Xai(OpenAiCompatClient),
    OpenAi(OpenAiCompatClient),
}

impl ProviderClient {
    pub fn from_model(model: &str) -> Result<Self, ApiError> {
        Self::from_model_with_provider_override(model, None)
    }

    pub fn from_model_with_anthropic_auth(
        model: &str,
        anthropic_auth: Option<AuthSource>,
    ) -> Result<Self, ApiError> {
        Self::from_model_with_anthropic_auth_resolver(model, None, || {
            anthropic_auth.ok_or_else(|| ApiError::Auth("missing anthropic auth".to_string()))
        })
    }

    pub fn from_model_with_provider_override(
        model: &str,
        provider_override: Option<&ProviderOverride>,
    ) -> Result<Self, ApiError> {
        Self::from_model_with_anthropic_auth_resolver(
            model,
            provider_override,
            anthropic::AuthSource::from_env_or_saved,
        )
    }

    pub fn from_model_with_anthropic_auth_resolver<F>(
        model: &str,
        provider_override: Option<&ProviderOverride>,
        resolve_anthropic_auth: F,
    ) -> Result<Self, ApiError>
    where
        F: FnOnce() -> Result<AuthSource, ApiError>,
    {
        let resolved_model = providers::resolve_model_alias(model);
        let provider_kind = provider_override
            .map(|config| config.provider)
            .unwrap_or_else(|| providers::detect_provider_kind(&resolved_model));
        match provider_kind {
            ProviderKind::Anthropic => Ok(Self::Anthropic(build_anthropic_client(
                provider_override,
                resolve_anthropic_auth,
            )?)),
            ProviderKind::Xai => Ok(Self::Xai(build_openai_compat_client(
                provider_override,
                OpenAiCompatConfig::xai(),
            )?)),
            ProviderKind::OpenAi => Ok(Self::OpenAi(build_openai_compat_client(
                provider_override,
                OpenAiCompatConfig::openai(),
            )?)),
        }
    }

    #[must_use]
    pub const fn provider_kind(&self) -> ProviderKind {
        match self {
            Self::Anthropic(_) => ProviderKind::Anthropic,
            Self::Xai(_) => ProviderKind::Xai,
            Self::OpenAi(_) => ProviderKind::OpenAi,
        }
    }

    #[must_use]
    pub fn with_prompt_cache(self, prompt_cache: PromptCache) -> Self {
        match self {
            Self::Anthropic(client) => Self::Anthropic(client.with_prompt_cache(prompt_cache)),
            other => other,
        }
    }

    #[must_use]
    pub fn prompt_cache_stats(&self) -> Option<PromptCacheStats> {
        match self {
            Self::Anthropic(client) => client.prompt_cache_stats(),
            Self::Xai(_) | Self::OpenAi(_) => None,
        }
    }

    #[must_use]
    pub fn take_last_prompt_cache_record(&self) -> Option<PromptCacheRecord> {
        match self {
            Self::Anthropic(client) => client.take_last_prompt_cache_record(),
            Self::Xai(_) | Self::OpenAi(_) => None,
        }
    }

    pub async fn send_message(
        &self,
        request: &MessageRequest,
    ) -> Result<MessageResponse, ApiError> {
        match self {
            Self::Anthropic(client) => send_via_provider(client, request).await,
            Self::Xai(client) | Self::OpenAi(client) => send_via_provider(client, request).await,
        }
    }

    pub async fn stream_message(
        &self,
        request: &MessageRequest,
    ) -> Result<MessageStream, ApiError> {
        match self {
            Self::Anthropic(client) => stream_via_provider(client, request)
                .await
                .map(MessageStream::Anthropic),
            Self::Xai(client) | Self::OpenAi(client) => stream_via_provider(client, request)
                .await
                .map(MessageStream::OpenAiCompat),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ProviderOverride {
    pub provider: ProviderKind,
    pub base_url: Option<String>,
    pub api_key_env: Option<String>,
    pub auth_token_env: Option<String>,
}

fn build_anthropic_client<F>(
    provider_override: Option<&ProviderOverride>,
    resolve_default_auth: F,
) -> Result<AnthropicClient, ApiError>
where
    F: FnOnce() -> Result<AuthSource, ApiError>,
{
    let Some(provider_override) = provider_override else {
        return Ok(
            AnthropicClient::from_auth(resolve_default_auth()?).with_base_url(read_base_url())
        );
    };

    let auth =
        if provider_override.api_key_env.is_none() && provider_override.auth_token_env.is_none() {
            resolve_default_auth()?
        } else {
            resolve_anthropic_auth_from_override(provider_override)?
        };
    let base_url = provider_override
        .base_url
        .clone()
        .unwrap_or_else(read_base_url);
    Ok(AnthropicClient::from_auth(auth).with_base_url(base_url))
}

fn build_openai_compat_client(
    provider_override: Option<&ProviderOverride>,
    config: OpenAiCompatConfig,
) -> Result<OpenAiCompatClient, ApiError> {
    let Some(provider_override) = provider_override else {
        return OpenAiCompatClient::from_env(config);
    };

    let api_key_env = provider_override
        .api_key_env
        .as_deref()
        .unwrap_or(config.api_key_env);
    let api_key = read_named_env_non_empty(api_key_env)?.ok_or_else(|| {
        ApiError::Auth(format!(
            "provider config requires credential environment variable {api_key_env}"
        ))
    })?;
    let base_url = provider_override
        .base_url
        .clone()
        .unwrap_or_else(|| openai_compat::read_base_url(config));
    Ok(OpenAiCompatClient::new(api_key, config).with_base_url(base_url))
}

fn resolve_anthropic_auth_from_override(
    provider_override: &ProviderOverride,
) -> Result<AuthSource, ApiError> {
    let api_key = provider_override
        .api_key_env
        .as_deref()
        .map(read_named_env_non_empty)
        .transpose()?
        .flatten();
    let auth_token = provider_override
        .auth_token_env
        .as_deref()
        .map(read_named_env_non_empty)
        .transpose()?
        .flatten();

    match (api_key, auth_token) {
        (Some(api_key), Some(bearer_token)) => Ok(AuthSource::ApiKeyAndBearer {
            api_key,
            bearer_token,
        }),
        (Some(api_key), None) => Ok(AuthSource::ApiKey(api_key)),
        (None, Some(bearer_token)) => Ok(AuthSource::BearerToken(bearer_token)),
        (None, None) => Err(ApiError::Auth(
            "provider config requires anthropic credentials from the configured env vars"
                .to_string(),
        )),
    }
}

fn read_named_env_non_empty(key: &str) -> Result<Option<String>, ApiError> {
    match std::env::var(key) {
        Ok(value) if !value.is_empty() => Ok(Some(value)),
        Ok(_) | Err(std::env::VarError::NotPresent) => Ok(None),
        Err(error) => Err(ApiError::from(error)),
    }
}

#[derive(Debug)]
pub enum MessageStream {
    Anthropic(anthropic::MessageStream),
    OpenAiCompat(openai_compat::MessageStream),
}

impl MessageStream {
    #[must_use]
    pub fn request_id(&self) -> Option<&str> {
        match self {
            Self::Anthropic(stream) => stream.request_id(),
            Self::OpenAiCompat(stream) => stream.request_id(),
        }
    }

    pub async fn next_event(&mut self) -> Result<Option<StreamEvent>, ApiError> {
        match self {
            Self::Anthropic(stream) => stream.next_event().await,
            Self::OpenAiCompat(stream) => stream.next_event().await,
        }
    }
}

pub use anthropic::{
    oauth_token_is_expired, resolve_saved_oauth_token, resolve_startup_auth_source, OAuthTokenSet,
};
#[must_use]
pub fn read_base_url() -> String {
    anthropic::read_base_url()
}

#[must_use]
pub fn read_xai_base_url() -> String {
    openai_compat::read_base_url(OpenAiCompatConfig::xai())
}

#[cfg(test)]
mod tests {
    use crate::providers::{detect_provider_kind, resolve_model_alias, ProviderKind};

    #[test]
    fn resolves_existing_and_grok_aliases() {
        assert_eq!(resolve_model_alias("opus"), "claude-opus-4-6");
        assert_eq!(resolve_model_alias("grok"), "grok-3");
        assert_eq!(resolve_model_alias("grok-mini"), "grok-3-mini");
    }

    #[test]
    fn provider_detection_prefers_model_family() {
        assert_eq!(detect_provider_kind("grok-3"), ProviderKind::Xai);
        assert_eq!(
            detect_provider_kind("claude-sonnet-4-6"),
            ProviderKind::Anthropic
        );
    }
}
