use std::fmt;

use serde::{
    Deserialize, Deserializer, Serialize, Serializer,
    de::{self, IntoDeserializer, Visitor},
};

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq, Default)]
pub enum Mode {
    /// Anthropic 接口，供 `claude_proxy` 直通转发
    #[serde(rename = "anthropic")]
    #[default]
    AnthropicDirect,
    /// `OpenAI` Responses 接口，供 `codex_proxy` 直通转发
    #[serde(rename = "openai_responses")]
    OpenAIResponses,
    /// `OpenAI` Chat Completions 接口（预留）
    #[serde(rename = "openai_chat")]
    OpenAIChat,
}

impl Mode {
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::AnthropicDirect => "anthropic",
            Self::OpenAIResponses => "openai_responses",
            Self::OpenAIChat => "openai_chat",
        }
    }
}

impl fmt::Display for Mode {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct UpstreamModes(Vec<Mode>);

impl UpstreamModes {
    #[must_use]
    pub fn supports(&self, mode: Mode) -> bool {
        self.0.contains(&mode)
    }

    fn normalize(modes: Vec<Mode>) -> Self {
        let mut normalized = Vec::with_capacity(modes.len());
        for mode in modes {
            if !normalized.contains(&mode) {
                normalized.push(mode);
            }
        }

        if normalized.is_empty() {
            return Self::default();
        }

        Self(normalized)
    }
}

impl Default for UpstreamModes {
    fn default() -> Self {
        Self(vec![Mode::AnthropicDirect])
    }
}

impl From<Vec<Mode>> for UpstreamModes {
    fn from(modes: Vec<Mode>) -> Self {
        Self::normalize(modes)
    }
}

impl Serialize for UpstreamModes {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        if self.0.len() == 1 {
            return self.0[0].serialize(serializer);
        }

        self.0.serialize(serializer)
    }
}

impl<'de> Deserialize<'de> for UpstreamModes {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        struct UpstreamModesVisitor;

        impl<'de> Visitor<'de> for UpstreamModesVisitor {
            type Value = UpstreamModes;

            fn expecting(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
                formatter.write_str("a mode string or a non-empty mode array")
            }

            fn visit_str<E>(self, value: &str) -> Result<Self::Value, E>
            where
                E: de::Error,
            {
                let mode = Mode::deserialize(value.into_deserializer())?;
                Ok(UpstreamModes::normalize(vec![mode]))
            }

            fn visit_seq<A>(self, seq: A) -> Result<Self::Value, A::Error>
            where
                A: de::SeqAccess<'de>,
            {
                let modes =
                    Vec::<Mode>::deserialize(serde::de::value::SeqAccessDeserializer::new(seq))?;
                if modes.is_empty() {
                    return Err(de::Error::custom("mode array must not be empty"));
                }

                Ok(UpstreamModes::normalize(modes))
            }
        }

        deserializer.deserialize_any(UpstreamModesVisitor)
    }
}

impl fmt::Display for UpstreamModes {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        if self.0.len() == 1 {
            return write!(f, "{}", self.0[0]);
        }

        let joined = self
            .0
            .iter()
            .map(|mode| mode.as_str())
            .collect::<Vec<_>>()
            .join(", ");
        write!(f, "[{joined}]")
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct GlobalUserAgentConfig {
    pub claude: Option<String>,
    pub codex: Option<String>,
}

impl GlobalUserAgentConfig {
    #[must_use]
    pub fn resolve_for_mode(&self, mode: Mode) -> Option<&str> {
        match mode {
            Mode::AnthropicDirect => self.claude.as_deref(),
            Mode::OpenAIResponses | Mode::OpenAIChat => self.codex.as_deref(),
        }
    }

    #[must_use]
    pub fn is_any_configured(&self) -> bool {
        [self.claude.as_deref(), self.codex.as_deref()]
            .into_iter()
            .flatten()
            .any(|value| !value.trim().is_empty())
    }
}

/// 上游提供商配置
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct UpstreamConfig {
    #[serde(default = "default_true")]
    pub enable: bool,
    #[serde(default)]
    pub name: String,
    #[serde(alias = "endpoint")]
    pub base_url: String,
    #[serde(
        default,
        deserialize_with = "deserialize_models",
        serialize_with = "serialize_models"
    )]
    pub model: Vec<String>,
    #[serde(default)]
    pub api_keys: Vec<String>,
    #[serde(default, alias = "ua_claude")]
    pub user_agent_claude: Option<String>,
    #[serde(default, alias = "ua_codex")]
    pub user_agent_codex: Option<String>,
    #[serde(default)]
    pub mode: UpstreamModes,
}

impl UpstreamConfig {
    #[must_use]
    pub fn user_agent_for_mode(&self, mode: Mode) -> Option<&str> {
        match mode {
            Mode::AnthropicDirect => self.user_agent_claude.as_deref(),
            Mode::OpenAIResponses | Mode::OpenAIChat => self.user_agent_codex.as_deref(),
        }
    }

    /// 检查 model 数组是否包含指定 model
    #[must_use]
    pub fn contains_model(&self, req_model: &str) -> bool {
        self.model.iter().any(|m| m == req_model)
    }
}

/// 反序列化 model 字段：接受字符串或字符串数组
fn deserialize_models<'de, D>(deserializer: D) -> Result<Vec<String>, D::Error>
where
    D: Deserializer<'de>,
{
    struct ModelsVisitor;

    impl<'de> Visitor<'de> for ModelsVisitor {
        type Value = Vec<String>;

        fn expecting(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
            formatter.write_str("a model string or an array of model strings")
        }

        fn visit_str<E>(self, value: &str) -> Result<Self::Value, E>
        where
            E: de::Error,
        {
            Ok(vec![value.to_owned()])
        }

        fn visit_seq<A>(self, seq: A) -> Result<Self::Value, A::Error>
        where
            A: de::SeqAccess<'de>,
        {
            Vec::<String>::deserialize(de::value::SeqAccessDeserializer::new(seq))
        }
    }

    deserializer.deserialize_any(ModelsVisitor)
}

/// 序列化 model 字段：单元素序列化为字符串，多元素序列化为数组
fn serialize_models<S>(models: &[String], serializer: S) -> Result<S::Ok, S::Error>
where
    S: Serializer,
{
    if models.len() == 1 {
        serializer.serialize_str(&models[0])
    } else {
        models.serialize(serializer)
    }
}

#[must_use]
pub const fn default_true() -> bool {
    true
}

#[must_use]
pub fn enabled_upstream_count(upstreams: &[UpstreamConfig]) -> usize {
    upstreams.iter().filter(|upstream| upstream.enable).count()
}
