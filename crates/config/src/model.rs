use my_selector::{GlobalUserAgentConfig, UpstreamConfig};
use serde::{Deserialize, Serialize};

/// 配置结构
#[derive(Debug, Clone)]
pub struct Config {
    pub server: ServerConfig,
    /// 上游提供商配置列表（支持多个上游负载均衡）
    pub upstream: Vec<UpstreamConfig>,
    /// 本地优化拦截开关
    pub optimizations: OptimizationConfig,
}

/// 服务运行配置
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct ServerConfig {
    /// 服务监听端口
    #[serde(default = "default_port")]
    pub port: u16,
    /// 强制轮询的 upstream 下标列表；非空时忽略 `enable` 字段，仅在列表内轮询
    #[serde(default)]
    pub force_upstream_index: Vec<usize>,
    /// 强制只使用 model 匹配此列表的 upstream；空列表时不生效
    #[serde(default)]
    pub force_model: Vec<String>,
    /// 是否打印请求体
    #[serde(default)]
    pub log_req_body: bool,
    /// 是否打印响应体
    #[serde(default)]
    pub log_res_body: bool,
    /// 仅用于 Claude 接口（Anthropic 模式）的全局 User-Agent
    #[serde(default)]
    pub user_agent_global_claude: Option<String>,
    /// 仅用于 Codex 接口（OpenAI Responses / Chat 模式）的全局 User-Agent
    #[serde(default)]
    pub user_agent_global_codex: Option<String>,
}

impl Default for ServerConfig {
    fn default() -> Self {
        Self {
            port: default_port(),
            force_upstream_index: vec![],
            force_model: vec![],
            log_req_body: false,
            log_res_body: false,
            user_agent_global_claude: None,
            user_agent_global_codex: None,
        }
    }
}

/// 本地优化配置
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct OptimizationConfig {
    #[serde(default = "default_true")]
    pub enable_network_probe_mock: bool,
    #[serde(default = "default_true")]
    pub enable_fast_prefix_detection: bool,
    #[serde(default = "default_true")]
    pub enable_historical_analysis_mock: bool,
    #[serde(default = "default_true")]
    pub enable_title_generation_skip: bool,
    #[serde(default = "default_true")]
    pub enable_suggestion_mode_skip: bool,
    #[serde(default = "default_true")]
    pub enable_filepath_extraction_mock: bool,
    #[serde(default = "default_true")]
    pub enable_strip_billing_header: bool,
}

impl Default for OptimizationConfig {
    fn default() -> Self {
        Self {
            enable_network_probe_mock: default_true(),
            enable_fast_prefix_detection: default_true(),
            enable_historical_analysis_mock: default_true(),
            enable_title_generation_skip: default_true(),
            enable_suggestion_mode_skip: default_true(),
            enable_filepath_extraction_mock: default_true(),
            enable_strip_billing_header: default_true(),
        }
    }
}

impl Config {
    #[must_use]
    pub fn global_user_agent_config(&self) -> GlobalUserAgentConfig {
        GlobalUserAgentConfig {
            claude: self.server.user_agent_global_claude.clone(),
            codex: self.server.user_agent_global_codex.clone(),
        }
    }
}

#[derive(Debug, Deserialize, Default)]
struct PartialConfig {
    #[serde(default)]
    server: PartialServerConfig,
    #[serde(default, flatten)]
    legacy_server: PartialServerConfig,
    #[serde(default)]
    upstream: Vec<UpstreamConfig>,
    #[serde(default)]
    optimizations: OptimizationConfig,
}

#[derive(Debug, Deserialize, Default)]
struct PartialServerConfig {
    #[serde(default)]
    port: Option<u16>,
    #[serde(default)]
    force_upstream_index: Option<Vec<usize>>,
    #[serde(default)]
    force_model: Option<Vec<String>>,
    #[serde(default)]
    log_req_body: Option<bool>,
    #[serde(default)]
    log_res_body: Option<bool>,
    #[serde(default)]
    user_agent_global_claude: Option<String>,
    #[serde(default)]
    user_agent_global_codex: Option<String>,
}

impl PartialServerConfig {
    fn merge(self, legacy: Self) -> ServerConfig {
        let defaults = ServerConfig::default();

        ServerConfig {
            port: self.port.or(legacy.port).unwrap_or(defaults.port),
            force_upstream_index: self
                .force_upstream_index
                .or(legacy.force_upstream_index)
                .unwrap_or(defaults.force_upstream_index),
            force_model: self
                .force_model
                .or(legacy.force_model)
                .unwrap_or(defaults.force_model),
            log_req_body: self
                .log_req_body
                .or(legacy.log_req_body)
                .unwrap_or(defaults.log_req_body),
            log_res_body: self
                .log_res_body
                .or(legacy.log_res_body)
                .unwrap_or(defaults.log_res_body),
            user_agent_global_claude: self
                .user_agent_global_claude
                .or(legacy.user_agent_global_claude),
            user_agent_global_codex: self
                .user_agent_global_codex
                .or(legacy.user_agent_global_codex),
        }
    }
}

impl<'de> Deserialize<'de> for Config {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        let partial = PartialConfig::deserialize(deserializer)?;

        Ok(Self {
            server: partial.server.merge(partial.legacy_server),
            upstream: partial.upstream,
            optimizations: partial.optimizations,
        })
    }
}

#[must_use]
pub const fn default_true() -> bool {
    true
}

#[must_use]
pub const fn default_port() -> u16 {
    9077
}

#[cfg(test)]
#[allow(clippy::unwrap_used)]
mod tests {
    use my_selector::{GlobalUserAgentConfig, Mode, UpstreamModes};

    use super::{Config, ServerConfig, default_port};

    #[test]
    fn upstream_enable_defaults_to_true() {
        let config: Config = toml::from_str(
            r#"
                [[upstream]]
                base_url = "https://example.com"
                model = "test-model"
                api_keys = ["test-key"]
                mode = "anthropic"
            "#,
        )
        .unwrap();

        assert_eq!(config.upstream.len(), 1);
        assert!(config.upstream[0].enable);
        assert_eq!(config.server.force_upstream_index, Vec::<usize>::new());
        assert!(config.upstream[0].name.is_empty());
        assert_eq!(config.upstream[0].mode, UpstreamModes::default());
    }

    #[test]
    fn server_force_upstream_index_deserializes_when_present() {
        let config: Config = toml::from_str(
            r"
                [server]
                force_upstream_index = [0, 2]
            ",
        )
        .unwrap();

        assert_eq!(config.server.force_upstream_index, vec![0, 2]);
    }

    #[test]
    fn upstream_name_deserializes_when_present() {
        let config: Config = toml::from_str(
            r#"
                [[upstream]]
                name = "primary-anthropic"
                base_url = "https://example.com"
                model = "test-model"
                api_keys = ["test-key"]
            "#,
        )
        .unwrap();

        assert_eq!(config.upstream[0].name, "primary-anthropic");
    }

    #[test]
    fn upstream_mode_accepts_array_and_deduplicates() {
        let config: Config = toml::from_str(
            r#"
                [[upstream]]
                base_url = "https://example.com"
                model = "test-model"
                api_keys = ["test-key"]
                mode = ["anthropic", "openai_responses", "anthropic"]
            "#,
        )
        .unwrap();

        assert_eq!(
            config.upstream[0].mode,
            vec![Mode::AnthropicDirect, Mode::OpenAIResponses].into()
        );
    }

    #[test]
    fn upstream_endpoint_alias_still_deserializes() {
        let config: Config = toml::from_str(
            r#"
                [[upstream]]
                endpoint = "https://legacy.example.com"
                model = "test-model"
                api_keys = ["test-key"]
            "#,
        )
        .unwrap();

        assert_eq!(config.upstream[0].base_url, "https://legacy.example.com");
    }

    #[test]
    fn upstream_mode_rejects_empty_array() {
        let result = toml::from_str::<Config>(
            r#"
                [[upstream]]
                base_url = "https://example.com"
                model = "test-model"
                api_keys = ["test-key"]
                mode = []
            "#,
        );

        assert!(result.is_err());
    }

    #[test]
    fn upstream_user_agent_fields_and_aliases_deserialize() {
        let cases = [
            r#"
                [[upstream]]
                base_url = "https://example.com"
                model = "test-model"
                api_keys = ["test-key"]
                user_agent_claude = "Claude-Code/1.0.84"
                user_agent_codex = "Codex/0.31.0"
            "#,
            r#"
                [[upstream]]
                base_url = "https://example.com"
                model = "test-model"
                api_keys = ["test-key"]
                ua_claude = "Claude-Code/1.0.84"
                ua_codex = "Codex/0.31.0"
            "#,
        ];

        for input in cases {
            let config: Config = toml::from_str(input).unwrap();

            assert_eq!(
                config.upstream[0].user_agent_claude.as_deref(),
                Some("Claude-Code/1.0.84")
            );
            assert_eq!(
                config.upstream[0].user_agent_codex.as_deref(),
                Some("Codex/0.31.0")
            );
        }
    }

    #[test]
    fn mode_specific_global_user_agents_deserialize_when_present() {
        let config: Config = toml::from_str(
            r#"
                [server]
                user_agent_global_claude = "Claude-Global/9.9.9"
                user_agent_global_codex = "Codex-Global/9.9.9"

                [[upstream]]
                base_url = "https://example.com"
                model = "test-model"
                api_keys = ["test-key"]
            "#,
        )
        .unwrap();

        assert_eq!(
            config.server.user_agent_global_claude.as_deref(),
            Some("Claude-Global/9.9.9")
        );
        assert_eq!(
            config.server.user_agent_global_codex.as_deref(),
            Some("Codex-Global/9.9.9")
        );
    }

    #[test]
    fn global_user_agent_config_reuses_codex_for_openai_chat() {
        let global = GlobalUserAgentConfig {
            claude: Some("Claude-Global/1.0".to_string()),
            codex: Some("Codex-Global/1.0".to_string()),
        };

        assert_eq!(
            global.resolve_for_mode(Mode::OpenAIResponses),
            Some("Codex-Global/1.0")
        );
        assert_eq!(
            global.resolve_for_mode(Mode::OpenAIChat),
            Some("Codex-Global/1.0")
        );
    }

    #[test]
    fn upstream_user_agent_reuses_codex_for_openai_chat() {
        let config: Config = toml::from_str(
            r#"
                [[upstream]]
                base_url = "https://example.com"
                model = "test-model"
                api_keys = ["test-key"]
                user_agent_claude = "Claude-Upstream/1.0"
                user_agent_codex = "Codex-Upstream/1.0"
            "#,
        )
        .unwrap();

        assert_eq!(
            config.upstream[0].user_agent_for_mode(Mode::OpenAIResponses),
            Some("Codex-Upstream/1.0")
        );
        assert_eq!(
            config.upstream[0].user_agent_for_mode(Mode::OpenAIChat),
            Some("Codex-Upstream/1.0")
        );
    }

    #[test]
    fn openai_chat_reuses_codex_user_agent_configuration() {
        let config: Config = toml::from_str(
            r#"
                [server]
                user_agent_global_codex = "Codex-Global/9.9.9"

                [[upstream]]
                base_url = "https://example.com"
                model = "test-model"
                api_keys = ["test-key"]
                user_agent_codex = "Codex-Upstream/1.0"
                mode = ["openai_chat"]
            "#,
        )
        .unwrap();

        let global_agents = config.global_user_agent_config();
        assert_eq!(
            global_agents.resolve_for_mode(Mode::OpenAIChat),
            Some("Codex-Global/9.9.9")
        );
        assert_eq!(
            config.upstream[0].user_agent_for_mode(Mode::OpenAIChat),
            Some("Codex-Upstream/1.0")
        );
    }

    #[test]
    fn port_uses_default_and_override_values() {
        let cases = [
            (
                r#"
                    [[upstream]]
                    base_url = "https://example.com"
                    model = "test-model"
                    api_keys = ["test-key"]
                "#,
                default_port(),
            ),
            (
                r#"
                    [server]
                    port = 19077

                    [[upstream]]
                    base_url = "https://example.com"
                    model = "test-model"
                    api_keys = ["test-key"]
                "#,
                19077,
            ),
        ];

        for (input, expected_port) in cases {
            let config: Config = toml::from_str(input).unwrap();
            assert_eq!(config.server.port, expected_port);
        }
    }

    #[test]
    fn legacy_top_level_server_fields_still_deserialize() {
        let config: Config = toml::from_str(
            r#"
                port = 19077
                log_req_body = true
                log_res_body = true
                user_agent_global_claude = "Claude-Global/9.9.9"
                user_agent_global_codex = "Codex-Global/9.9.9"

                [[upstream]]
                base_url = "https://example.com"
                model = "test-model"
                api_keys = ["test-key"]
            "#,
        )
        .unwrap();

        assert_eq!(
            config.server,
            ServerConfig {
                port: 19077,
                force_upstream_index: vec![],
                force_model: vec![],
                log_req_body: true,
                log_res_body: true,
                user_agent_global_claude: Some("Claude-Global/9.9.9".to_string()),
                user_agent_global_codex: Some("Codex-Global/9.9.9".to_string()),
            }
        );
    }

    #[test]
    fn nested_server_fields_override_legacy_top_level_values() {
        let config: Config = toml::from_str(
            r#"
                port = 19077
                log_req_body = false

                [server]
                port = 29077
                log_req_body = true

                [[upstream]]
                base_url = "https://example.com"
                model = "test-model"
                api_keys = ["test-key"]
            "#,
        )
        .unwrap();

        assert_eq!(config.server.port, 29077);
        assert!(config.server.log_req_body);
    }
}
