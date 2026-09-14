//! Upstream 轮询选择器
//!
//! 使用双层 round-robin 策略：
//! 1. 外层：遍历每个 upstream
//! 2. 内层：在每个 upstream 内部遍历其 `api_keys`
//!    即：upstream[0].key[0] -> upstream[0].key[1] -> ... -> upstream[1].key[0] -> ...

use std::sync::atomic::{AtomicUsize, Ordering};
use tracing::info;

use super::{Mode, UpstreamConfig, model::GlobalUserAgentConfig};

type UpstreamSelection<'a> = (
    usize,
    &'a str,
    &'a str,
    &'a str,
    &'a str,
    Option<&'a str>,
    Mode,
);

/// Upstream 选择器，使用双层 round-robin 策略
pub struct UpstreamSelector {
    /// 上游配置列表
    upstreams: Vec<UpstreamConfig>,
    /// 按接口区分的全局默认 User-Agent
    global_user_agents: GlobalUserAgentConfig,
    /// 强制轮询的 upstream 下标列表；非空时忽略 `enable` 字段
    force_upstream_index: Vec<usize>,
    /// `anthropic` 模式独立轮询计数
    next_index_anthropic: AtomicUsize,
    /// `openai_responses` 模式独立轮询计数
    next_index_openai_responses: AtomicUsize,
    /// `openai_chat` 模式独立轮询计数
    next_index_openai_chat: AtomicUsize,
}

impl UpstreamSelector {
    /// 创建新的 Upstream 选择器
    #[cfg(test)]
    #[must_use]
    pub fn new(global_user_agent: Option<String>, upstreams: Vec<UpstreamConfig>) -> Option<Self> {
        Self::new_with_global_user_agents(
            GlobalUserAgentConfig {
                claude: global_user_agent,
                codex: None,
            },
            vec![],
            upstreams,
        )
    }

    #[must_use]
    pub fn new_with_global_user_agents(
        global_user_agents: GlobalUserAgentConfig,
        force_upstream_index: Vec<usize>,
        upstreams: Vec<UpstreamConfig>,
    ) -> Option<Self> {
        if upstreams.is_empty() {
            return None;
        }
        Some(Self {
            upstreams,
            global_user_agents,
            force_upstream_index,
            next_index_anthropic: AtomicUsize::new(0),
            next_index_openai_responses: AtomicUsize::new(0),
            next_index_openai_chat: AtomicUsize::new(0),
        })
    }

    const fn mode_counter(&self, mode: Mode) -> &AtomicUsize {
        match mode {
            Mode::AnthropicDirect => &self.next_index_anthropic,
            Mode::OpenAIResponses => &self.next_index_openai_responses,
            Mode::OpenAIChat => &self.next_index_openai_chat,
        }
    }

    fn resolve_user_agent<'a>(
        &'a self,
        upstream: &'a UpstreamConfig,
        expected_mode: Mode,
    ) -> Option<&'a str> {
        upstream
            .user_agent_for_mode(expected_mode)
            .or_else(|| self.global_user_agents.resolve_for_mode(expected_mode))
    }

    fn forced_upstream_for_mode_and_model(
        &self,
        expected_mode: Mode,
        request_model: Option<&str>,
    ) -> Option<(usize, &UpstreamConfig)> {
        if self.force_upstream_index.is_empty() {
            return None;
        }
        let mode_idx = self.mode_counter(expected_mode).load(Ordering::Relaxed);
        let len = self.force_upstream_index.len();
        for i in 0..len {
            let pos = (mode_idx + i) % len;
            let index = *self.force_upstream_index.get(pos)?;
            if let Some(upstream) = self.upstreams.get(index)
                && upstream.mode.supports(expected_mode)
            {
                // 如果指定了 request_model，则必须匹配
                if let Some(req_model) = request_model
                    && upstream.model != req_model
                {
                    info!("传入的{req_model}不匹配，将覆盖为{0:?}", upstream.model);
                }
                return Some((index, upstream));
            }
        }
        None
    }

    /// 检查 upstream 是否匹配指定的 mode
    #[must_use]
    fn matches_mode(upstream: &UpstreamConfig, expected_mode: Mode) -> bool {
        upstream.mode.supports(expected_mode)
    }

    /// 获取指定 mode 当前可用的 upstream 数量
    pub fn matching_count_by_mode(&self, expected_mode: Mode) -> usize {
        self.matching_count_by_mode_and_model(expected_mode, None)
    }

    /// 获取指定 mode 和 model 当前可用的 upstream 数量
    /// 当 `request_model` 为 Some 时，只统计 upstream.model 与其相同的上游
    pub fn matching_count_by_mode_and_model(
        &self,
        expected_mode: Mode,
        request_model: Option<&str>,
    ) -> usize {
        if !self.force_upstream_index.is_empty() {
            return self
                .force_upstream_index
                .iter()
                .filter(|&&idx| {
                    self.upstreams.get(idx).is_some_and(|u| {
                        Self::matches_mode(u, expected_mode)
                            && request_model.is_none_or(|req_model| u.model == req_model)
                    })
                })
                .count();
        }

        self.upstreams
            .iter()
            .filter(|upstream| {
                upstream.enable
                    && Self::matches_mode(upstream, expected_mode)
                    && request_model.is_none_or(|req_model| upstream.model == req_model)
            })
            .count()
    }

    /// 获取下一个匹配指定 mode 的 upstream 和对应的 `api_key`
    /// 双层轮询策略：
    /// 1. 外层：按 round-robin 选择 upstream
    /// 2. 内层：在该 upstream 内部按 round-robin 选择 `api_key`
    ///
    /// 例如：2个upstream，每个有3个key
    /// 请求1: upstream[0], key[0]
    /// 请求2: upstream[1], key[0]
    /// 请求3: upstream[0], key[1]
    /// 请求4: upstream[1], key[1]
    /// 请求5: upstream[0], key[2]
    /// 请求6: upstream[1], key[2]
    /// 请求7: upstream[0], key[0]  (循环)
    ///
    /// 如果提供了 `request_model`，则只在 upstream.model 与其相同的上游之间轮询
    ///
    /// 返回 (upstream索引, `name`, `base_url`, model, `api_key`, `user_agent`, `mode`)
    ///
    pub fn next_by_mode(&self, expected_mode: Mode) -> Option<UpstreamSelection<'_>> {
        self.next_by_mode_and_model(expected_mode, None)
    }

    /// 获取下一个匹配指定 mode 和 model 的 upstream
    /// 当 `request_model` 为 Some 时，只在 upstream.model 与其相同的上游之间轮询
    pub fn next_by_mode_and_model(
        &self,
        expected_mode: Mode,
        request_model: Option<&str>,
    ) -> Option<UpstreamSelection<'_>> {
        if let Some((upstream_idx, upstream)) =
            self.forced_upstream_for_mode_and_model(expected_mode, request_model)
        {
            let mode_idx = self
                .mode_counter(expected_mode)
                .fetch_add(1, Ordering::Relaxed);
            let api_key = if upstream.api_keys.is_empty() {
                ""
            } else {
                let key_idx = mode_idx % upstream.api_keys.len();
                &upstream.api_keys[key_idx]
            };

            return Some((
                upstream_idx,
                &upstream.name,
                &upstream.base_url,
                &upstream.model,
                api_key,
                self.resolve_user_agent(upstream, expected_mode),
                expected_mode,
            ));
        }

        let matching_count = self.matching_count_by_mode_and_model(expected_mode, request_model);

        if matching_count == 0 {
            return None;
        }

        let mode_idx = self
            .mode_counter(expected_mode)
            .fetch_add(1, Ordering::Relaxed);
        let target_pos = mode_idx % matching_count;

        let mut seen = 0;
        let (upstream_idx, upstream) =
            self.upstreams.iter().enumerate().find(|(_, upstream)| {
                if !upstream.enable || !Self::matches_mode(upstream, expected_mode) {
                    return false;
                }

                // 额外检查请求 model 是否匹配
                if let Some(req_model) = request_model
                    && upstream.model != req_model
                {
                    return false;
                }

                let is_target = seen == target_pos;
                seen += 1;
                is_target
            })?;

        let api_key = if upstream.api_keys.is_empty() {
            ""
        } else {
            let key_count = upstream.api_keys.len();
            let key_idx = (mode_idx / matching_count) % key_count;
            &upstream.api_keys[key_idx]
        };

        Some((
            upstream_idx,
            &upstream.name,
            &upstream.base_url,
            &upstream.model,
            api_key,
            self.resolve_user_agent(upstream, expected_mode),
            expected_mode,
        ))
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used)]
mod tests {
    use super::*;

    fn create_test_upstreams() -> Vec<UpstreamConfig> {
        vec![
            UpstreamConfig {
                enable: true,
                name: "upstream-1".to_string(),
                base_url: "https://upstream1.example.com".to_string(),
                model: "model1".to_string(),
                api_keys: vec!["key1a".to_string(), "key1b".to_string()],
                user_agent_claude: Some("Device-A/1.0".to_string()),
                user_agent_codex: None,
                mode: vec![Mode::AnthropicDirect].into(),
            },
            UpstreamConfig {
                enable: true,
                name: "upstream-2".to_string(),
                base_url: "https://upstream2.example.com".to_string(),
                model: "model2".to_string(),
                api_keys: vec![
                    "key2a".to_string(),
                    "key2b".to_string(),
                    "key2c".to_string(),
                ],
                user_agent_claude: None,
                user_agent_codex: None,
                mode: vec![Mode::OpenAIResponses].into(),
            },
        ]
    }

    #[test]
    fn test_empty_upstreams_returns_none() {
        let selector = UpstreamSelector::new(None, Vec::new());
        // new() 返回 None 当输入为空时
        assert!(selector.is_none());
    }

    #[test]
    fn test_round_robin_and_mode_filtering() {
        let upstreams = vec![
            UpstreamConfig {
                enable: true,
                name: "upstream-1".to_string(),
                base_url: "https://upstream1.example.com".to_string(),
                model: "model1".to_string(),
                api_keys: vec!["key1a".to_string()],
                user_agent_claude: None,
                user_agent_codex: None,
                mode: vec![Mode::AnthropicDirect].into(),
            },
            UpstreamConfig {
                enable: true,
                name: "upstream-2".to_string(),
                base_url: "https://upstream2.example.com".to_string(),
                model: "model2".to_string(),
                api_keys: vec!["key2a".to_string(), "key2b".to_string()],
                user_agent_claude: None,
                user_agent_codex: None,
                mode: vec![Mode::OpenAIResponses].into(),
            },
            UpstreamConfig {
                enable: true,
                name: "upstream-3".to_string(),
                base_url: "https://upstream3.example.com".to_string(),
                model: "model3".to_string(),
                api_keys: vec!["key3a".to_string(), "key3b".to_string()],
                user_agent_claude: None,
                user_agent_codex: None,
                mode: vec![Mode::OpenAIResponses].into(),
            },
        ];
        let selector =
            UpstreamSelector::new(None, upstreams).expect("测试数据已确保 upstreams 非空");

        // 验证轮询顺序：upstream[1] key[0] -> upstream[2] key[0] -> upstream[1] key[1] -> upstream[2] key[1]
        let (idx0, _, _, _, key0, _, mode0) = selector
            .next_by_mode(Mode::OpenAIResponses)
            .expect("应能选到第一个匹配 upstream");
        assert_eq!((idx0, key0, mode0), (1, "key2a", Mode::OpenAIResponses));

        let (idx1, _, _, _, key1, _, _) = selector
            .next_by_mode(Mode::OpenAIResponses)
            .expect("应轮询到下一个 upstream");
        assert_eq!((idx1, key1), (2, "key3a"));

        let (idx2, _, _, _, key2, _, _) = selector
            .next_by_mode(Mode::OpenAIResponses)
            .expect("应回到第一个 upstream 的下一个 key");
        assert_eq!((idx2, key2), (1, "key2b"));

        let (idx3, _, _, _, key3, _, _) = selector
            .next_by_mode(Mode::OpenAIResponses)
            .expect("应轮询第二个 upstream 的下一个 key");
        assert_eq!((idx3, key3), (2, "key3b"));
    }

    #[test]
    fn test_disabled_upstreams_handling() {
        let upstreams = vec![
            UpstreamConfig {
                enable: false,
                name: "disabled-upstream".to_string(),
                base_url: "https://disabled.example.com".to_string(),
                model: "disabled-model".to_string(),
                api_keys: vec!["disabled-key".to_string()],
                user_agent_claude: None,
                user_agent_codex: None,
                mode: vec![Mode::OpenAIResponses].into(),
            },
            UpstreamConfig {
                enable: true,
                name: "enabled-upstream".to_string(),
                base_url: "https://enabled.example.com".to_string(),
                model: "enabled-model".to_string(),
                api_keys: vec!["enabled-key".to_string()],
                user_agent_claude: None,
                user_agent_codex: None,
                mode: vec![Mode::OpenAIResponses].into(),
            },
        ];
        let selector =
            UpstreamSelector::new(None, upstreams).expect("测试数据已确保 upstreams 非空");

        // 应跳过禁用项
        let (idx, _, _, _, _, _, _) = selector
            .next_by_mode(Mode::OpenAIResponses)
            .expect("应跳过禁用 upstream");
        assert_eq!(idx, 1);

        // 全部禁用时返回 None
        let all_disabled = vec![UpstreamConfig {
            enable: false,
            name: "disabled".to_string(),
            base_url: "https://disabled.example.com".to_string(),
            model: "model".to_string(),
            api_keys: vec!["key".to_string()],
            user_agent_claude: None,
            user_agent_codex: None,
            mode: vec![Mode::OpenAIResponses].into(),
        }];
        let selector2 = UpstreamSelector::new(None, all_disabled).expect("upstreams 非空");
        assert!(selector2.next_by_mode(Mode::OpenAIResponses).is_none());
    }

    #[test]
    fn test_multi_mode_upstream_support() {
        let upstreams = vec![
            UpstreamConfig {
                enable: true,
                name: "shared-upstream".to_string(),
                base_url: "https://multi.example.com".to_string(),
                model: "shared-model".to_string(),
                api_keys: vec!["shared-key-1".to_string(), "shared-key-2".to_string()],
                user_agent_claude: Some("Claude-UA/1.0".to_string()),
                user_agent_codex: Some("Codex-UA/1.0".to_string()),
                mode: vec![Mode::AnthropicDirect, Mode::OpenAIResponses].into(),
            },
            UpstreamConfig {
                enable: true,
                name: "responses-only".to_string(),
                base_url: "https://responses.example.com".to_string(),
                model: "responses-model".to_string(),
                api_keys: vec!["responses-key".to_string()],
                user_agent_claude: None,
                user_agent_codex: None,
                mode: vec![Mode::OpenAIResponses].into(),
            },
        ];
        let selector =
            UpstreamSelector::new(None, upstreams).expect("测试数据已确保 upstreams 非空");

        // 验证多协议 upstream 支持 AnthropicDirect
        let (idx, _, _, _, key, user_agent, mode) = selector
            .next_by_mode(Mode::AnthropicDirect)
            .expect("多协议 upstream 应支持 anthropic");
        assert_eq!(
            (idx, key, user_agent, mode),
            (
                0,
                "shared-key-1",
                Some("Claude-UA/1.0"),
                Mode::AnthropicDirect
            )
        );

        // 验证多协议 upstream 也支持 OpenAIResponses
        let (idx, _, _, _, key, user_agent, mode) = selector
            .next_by_mode(Mode::OpenAIResponses)
            .expect("多协议 upstream 应支持 openai_responses");
        assert_eq!(
            (idx, key, user_agent, mode),
            (
                0,
                "shared-key-1",
                Some("Codex-UA/1.0"),
                Mode::OpenAIResponses
            )
        );

        // 验证计数包含多协议 upstream
        assert_eq!(selector.matching_count_by_mode(Mode::AnthropicDirect), 1);
        assert_eq!(selector.matching_count_by_mode(Mode::OpenAIResponses), 2);
    }

    #[test]
    fn test_force_upstream_index_core_logic() {
        let upstreams = vec![
            UpstreamConfig {
                enable: true,
                name: "first-upstream".to_string(),
                base_url: "https://first.example.com".to_string(),
                model: "model-1".to_string(),
                api_keys: vec!["key-1a".to_string(), "key-1b".to_string()],
                user_agent_claude: None,
                user_agent_codex: None,
                mode: vec![Mode::AnthropicDirect].into(),
            },
            UpstreamConfig {
                enable: false,
                name: "forced-upstream".to_string(),
                base_url: "https://forced.example.com".to_string(),
                model: "model-2".to_string(),
                api_keys: vec!["key-2a".to_string(), "key-2b".to_string()],
                user_agent_claude: Some("Forced-UA/1.0".to_string()),
                user_agent_codex: None,
                mode: vec![Mode::AnthropicDirect].into(),
            },
        ];
        let selector = UpstreamSelector::new_with_global_user_agents(
            GlobalUserAgentConfig::default(),
            vec![1],
            upstreams,
        )
        .expect("测试数据已确保 upstreams 非空");

        // 强制索引忽略 enable 标志
        assert_eq!(selector.matching_count_by_mode(Mode::AnthropicDirect), 1);

        // 验证只在指定 upstream 的 keys 内轮询
        let first = selector
            .next_by_mode(Mode::AnthropicDirect)
            .expect("应命中强制索引");
        let second = selector
            .next_by_mode(Mode::AnthropicDirect)
            .expect("应继续轮询 keys");
        let third = selector
            .next_by_mode(Mode::AnthropicDirect)
            .expect("应回到第一个 key");

        assert_eq!((first.0, first.4), (1, "key-2a"));
        assert_eq!((second.0, second.4), (1, "key-2b"));
        assert_eq!((third.0, third.4), (1, "key-2a"));

        // 验证强制索引越界返回 None
        let out_of_range = UpstreamSelector::new_with_global_user_agents(
            GlobalUserAgentConfig::default(),
            vec![5],
            create_test_upstreams(),
        )
        .expect("upstreams 非空");
        assert!(out_of_range.next_by_mode(Mode::AnthropicDirect).is_none());
    }

    #[test]
    fn test_force_upstream_index_mode_support() {
        let upstreams = vec![
            UpstreamConfig {
                enable: true,
                name: "openai-only".to_string(),
                base_url: "https://openai.example.com".to_string(),
                model: "model-o".to_string(),
                api_keys: vec!["key-o".to_string()],
                user_agent_claude: None,
                user_agent_codex: None,
                mode: vec![Mode::OpenAIResponses].into(),
            },
            UpstreamConfig {
                enable: true,
                name: "anthropic-upstream".to_string(),
                base_url: "https://anthropic.example.com".to_string(),
                model: "model-a".to_string(),
                api_keys: vec!["key-a".to_string()],
                user_agent_claude: None,
                user_agent_codex: None,
                mode: vec![Mode::AnthropicDirect].into(),
            },
        ];
        let selector = UpstreamSelector::new_with_global_user_agents(
            GlobalUserAgentConfig::default(),
            vec![0, 1],
            upstreams,
        )
        .expect("测试数据已确保 upstreams 非空");

        // 强制索引仍需遵循 mode 支持，upstream[0] 不支持 AnthropicDirect
        let (idx, _, _, _, key, _, _) = selector
            .next_by_mode(Mode::AnthropicDirect)
            .expect("应跳过不支持的 upstream[0]");
        assert_eq!((idx, key), (1, "key-a"));

        // 验证不支持的 mode 返回 None
        let single_mode_selector = UpstreamSelector::new_with_global_user_agents(
            GlobalUserAgentConfig::default(),
            vec![1],
            vec![
                UpstreamConfig {
                    enable: true,
                    name: "anthropic-only".to_string(),
                    base_url: "https://anthropic.example.com".to_string(),
                    model: "model-a".to_string(),
                    api_keys: vec!["key-a".to_string()],
                    user_agent_claude: None,
                    user_agent_codex: None,
                    mode: vec![Mode::AnthropicDirect].into(),
                },
                UpstreamConfig {
                    enable: false,
                    name: "responses-upstream".to_string(),
                    base_url: "https://responses.example.com".to_string(),
                    model: "model-r".to_string(),
                    api_keys: vec!["key-r".to_string()],
                    user_agent_claude: None,
                    user_agent_codex: None,
                    mode: vec![Mode::OpenAIResponses].into(),
                },
            ],
        )
        .expect("upstreams 非空");
        assert!(
            single_mode_selector
                .next_by_mode(Mode::AnthropicDirect)
                .is_none()
        );
    }

    #[test]
    fn test_next_by_mode_resolves_user_agent_priority() {
        struct Case {
            name: &'static str,
            mode: Mode,
            upstream_mode: Vec<Mode>,
            upstream_claude: Option<&'static str>,
            upstream_codex: Option<&'static str>,
            global_claude: Option<&'static str>,
            global_codex: Option<&'static str>,
            expected_user_agent: Option<&'static str>,
        }

        let cases = [
            Case {
                name: "uses upstream anthropic user agent",
                mode: Mode::AnthropicDirect,
                upstream_mode: vec![Mode::AnthropicDirect],
                upstream_claude: Some("Device-D/1.0"),
                upstream_codex: None,
                global_claude: None,
                global_codex: None,
                expected_user_agent: Some("Device-D/1.0"),
            },
            Case {
                name: "falls back to global anthropic user agent",
                mode: Mode::AnthropicDirect,
                upstream_mode: vec![Mode::AnthropicDirect],
                upstream_claude: None,
                upstream_codex: None,
                global_claude: Some("Global-UA/1.0"),
                global_codex: None,
                expected_user_agent: Some("Global-UA/1.0"),
            },
            Case {
                name: "prefers upstream anthropic user agent over global",
                mode: Mode::AnthropicDirect,
                upstream_mode: vec![Mode::AnthropicDirect],
                upstream_claude: Some("Upstream-UA/2.0"),
                upstream_codex: None,
                global_claude: Some("Global-UA/1.0"),
                global_codex: None,
                expected_user_agent: Some("Upstream-UA/2.0"),
            },
            Case {
                name: "prefers upstream codex user agent over global responses",
                mode: Mode::OpenAIResponses,
                upstream_mode: vec![Mode::AnthropicDirect, Mode::OpenAIResponses],
                upstream_claude: Some("Claude-Upstream-UA/1.0"),
                upstream_codex: Some("Codex-Upstream-UA/1.0"),
                global_claude: Some("Claude-Global-UA/2.0"),
                global_codex: Some("Codex-Global-UA/3.0"),
                expected_user_agent: Some("Codex-Upstream-UA/1.0"),
            },
            Case {
                name: "uses mode-specific global codex user agent",
                mode: Mode::OpenAIResponses,
                upstream_mode: vec![Mode::AnthropicDirect, Mode::OpenAIResponses],
                upstream_claude: None,
                upstream_codex: None,
                global_claude: Some("Claude-Global-UA/2.0"),
                global_codex: Some("Codex-Global-UA/3.0"),
                expected_user_agent: Some("Codex-Global-UA/3.0"),
            },
            Case {
                name: "uses codex global user agent for openai chat",
                mode: Mode::OpenAIChat,
                upstream_mode: vec![Mode::OpenAIChat],
                upstream_claude: None,
                upstream_codex: None,
                global_claude: Some("Claude-Global-UA/2.0"),
                global_codex: Some("Codex-Global-UA/3.0"),
                expected_user_agent: Some("Codex-Global-UA/3.0"),
            },
        ];

        for case in cases {
            let selector = UpstreamSelector::new_with_global_user_agents(
                GlobalUserAgentConfig {
                    claude: case.global_claude.map(str::to_owned),
                    codex: case.global_codex.map(str::to_owned),
                },
                vec![],
                vec![UpstreamConfig {
                    enable: true,
                    name: "ua-upstream".to_string(),
                    base_url: "https://ua.example.com".to_string(),
                    model: "ua-model".to_string(),
                    api_keys: vec!["ua-key".to_string()],
                    user_agent_claude: case.upstream_claude.map(str::to_owned),
                    user_agent_codex: case.upstream_codex.map(str::to_owned),
                    mode: case.upstream_mode.into(),
                }],
            )
            .expect("测试数据已确保 upstreams 非空");

            let (_, _, _, _, _, user_agent, mode) = selector
                .next_by_mode(case.mode)
                .expect("应能返回匹配 mode 的 upstream");

            assert_eq!(mode, case.mode, "{}", case.name);
            assert_eq!(user_agent, case.expected_user_agent, "{}", case.name);
        }
    }
}
