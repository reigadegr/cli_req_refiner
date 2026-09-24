//! Upstream 与 `api_key` 轮询选择器
//!
//! - upstream：按请求 round-robin 轮询选择（[`UpstreamSelector::next_by_mode_and_model`]）
//! - `api_key`：按尝试 round-robin 轮换（[`UpstreamSelector::next_api_key`]），
//!   因此不论请求成功还是失败重试，每一次尝试都会使用下一个 key

use std::sync::atomic::{AtomicUsize, Ordering};
use tracing::info;

use super::{Mode, UpstreamConfig, model::GlobalUserAgentConfig};

type UpstreamSelection<'a> = (usize, &'a str, &'a str, &'a [String], Option<&'a str>, Mode);

/// Upstream 选择器，upstream 按请求轮询，`api_key` 按尝试轮换
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
    /// 每个 upstream 独立的 `api_key` 轮询计数，每次尝试（成功或失败重试）都推进一次
    next_key_index: Vec<AtomicUsize>,
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
        let next_key_index = upstreams.iter().map(|_| AtomicUsize::new(0)).collect();
        Some(Self {
            upstreams,
            global_user_agents,
            force_upstream_index,
            next_index_anthropic: AtomicUsize::new(0),
            next_index_openai_responses: AtomicUsize::new(0),
            next_index_openai_chat: AtomicUsize::new(0),
            next_key_index,
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
            let index = self.force_upstream_index[pos];
            if let Some(upstream) = self.upstreams.get(index)
                && upstream.mode.supports(expected_mode)
            {
                // 如果指定了 request_model，则必须匹配
                if let Some(req_model) = request_model
                    && !upstream.contains_model(req_model)
                {
                    info!("传入的{req_model}不匹配，将覆盖为{:?}", upstream.model);
                }
                return Some((index, upstream));
            }
        }
        None
    }

    /// 获取指定 mode 当前可用的 upstream 数量
    pub fn matching_count_by_mode(&self, expected_mode: Mode) -> usize {
        self.matching_count_by_mode_and_model(expected_mode, None)
    }

    /// 获取指定 mode 和 model 当前可用的 upstream 数量
    /// 当 `request_model` 为 Some 时，只统计 model 数组包含该值的上游
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
                    self.upstreams
                        .get(idx)
                        .is_some_and(|u| u.mode.supports(expected_mode))
                })
                .count();
        }

        self.upstreams
            .iter()
            .filter(|upstream| {
                upstream.enable
                    && upstream.mode.supports(expected_mode)
                    && request_model.is_none_or(|req_model| upstream.contains_model(req_model))
            })
            .count()
    }

    /// 获取指定 upstream 下一次尝试应使用的 `api_key`
    ///
    /// 每个 upstream 有独立的轮询计数，每次调用都会推进，
    /// 因此 key 的轮换只取决于尝试次数：不论请求成功还是失败重试，每次尝试都会换到下一个 key。
    pub fn next_api_key(&self, upstream_index: usize) -> &str {
        let Some(upstream) = self.upstreams.get(upstream_index) else {
            return "";
        };
        if upstream.api_keys.is_empty() {
            return "";
        }

        let key_index = self.next_key_index[upstream_index].fetch_add(1, Ordering::Relaxed)
            % upstream.api_keys.len();
        &upstream.api_keys[key_index]
    }

    /// 获取下一个匹配指定 mode 的 upstream
    ///
    /// upstream 按请求 round-robin 轮询；`api_key` 不在本次选择中决定，
    /// 而是由 [`Self::next_api_key`] 在每次尝试时轮换。
    ///
    /// 如果提供了 `request_model`，则只在 model 数组包含该值的上游之间轮询
    ///
    /// 返回 (upstream索引, `name`, `base_url`, models, `user_agent`, `mode`)
    ///
    pub fn next_by_mode(&self, expected_mode: Mode) -> Option<UpstreamSelection<'_>> {
        self.next_by_mode_and_model(expected_mode, None)
    }

    /// 获取下一个匹配指定 mode 和 model 的 upstream
    /// 当 `request_model` 为 Some 时，只在 model 数组包含该值的上游之间轮询
    pub fn next_by_mode_and_model(
        &self,
        expected_mode: Mode,
        request_model: Option<&str>,
    ) -> Option<UpstreamSelection<'_>> {
        if let Some((upstream_idx, upstream)) =
            self.forced_upstream_for_mode_and_model(expected_mode, request_model)
        {
            self.mode_counter(expected_mode)
                .fetch_add(1, Ordering::Relaxed);

            return Some((
                upstream_idx,
                &upstream.name,
                &upstream.base_url,
                &upstream.model,
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
                let matches = upstream.enable
                    && upstream.mode.supports(expected_mode)
                    && request_model.is_none_or(|req_model| upstream.contains_model(req_model));
                if !matches {
                    return false;
                }

                let is_target = seen == target_pos;
                seen += 1;
                is_target
            })?;

        Some((
            upstream_idx,
            &upstream.name,
            &upstream.base_url,
            &upstream.model,
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
                model: vec!["model1".to_string()],
                api_keys: vec!["key1a".to_string(), "key1b".to_string()],
                user_agent_claude: Some("Device-A/1.0".to_string()),
                user_agent_codex: None,
                mode: vec![Mode::AnthropicDirect].into(),
            },
            UpstreamConfig {
                enable: true,
                name: "upstream-2".to_string(),
                base_url: "https://upstream2.example.com".to_string(),
                model: vec!["model2".to_string()],
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
                model: vec!["model1".to_string()],
                api_keys: vec!["key1a".to_string()],
                user_agent_claude: None,
                user_agent_codex: None,
                mode: vec![Mode::AnthropicDirect].into(),
            },
            UpstreamConfig {
                enable: true,
                name: "upstream-2".to_string(),
                base_url: "https://upstream2.example.com".to_string(),
                model: vec!["model2".to_string()],
                api_keys: vec!["key2a".to_string(), "key2b".to_string()],
                user_agent_claude: None,
                user_agent_codex: None,
                mode: vec![Mode::OpenAIResponses].into(),
            },
            UpstreamConfig {
                enable: true,
                name: "upstream-3".to_string(),
                base_url: "https://upstream3.example.com".to_string(),
                model: vec!["model3".to_string()],
                api_keys: vec!["key3a".to_string(), "key3b".to_string()],
                user_agent_claude: None,
                user_agent_codex: None,
                mode: vec![Mode::OpenAIResponses].into(),
            },
        ];
        let selector =
            UpstreamSelector::new(None, upstreams).expect("测试数据已确保 upstreams 非空");

        // 验证 upstream 轮询顺序：upstream[1] -> upstream[2] -> upstream[1] -> upstream[2]
        // api_key 不在选择 upstream 时决定，改由 next_api_key 按尝试轮换
        let (idx0, _, _, _, _, mode0) = selector
            .next_by_mode(Mode::OpenAIResponses)
            .expect("应能选到第一个匹配 upstream");
        assert_eq!((idx0, mode0), (1, Mode::OpenAIResponses));

        let (idx1, _, _, _, _, _) = selector
            .next_by_mode(Mode::OpenAIResponses)
            .expect("应轮询到下一个 upstream");
        assert_eq!(idx1, 2);

        let (idx2, _, _, _, _, _) = selector
            .next_by_mode(Mode::OpenAIResponses)
            .expect("应回到第一个 upstream");
        assert_eq!(idx2, 1);

        let (idx3, _, _, _, _, _) = selector
            .next_by_mode(Mode::OpenAIResponses)
            .expect("应轮询第二个 upstream");
        assert_eq!(idx3, 2);
    }

    #[test]
    fn test_next_api_key_rotates_per_attempt() {
        let upstreams = vec![
            UpstreamConfig {
                enable: true,
                name: "two-keys".to_string(),
                base_url: "https://two.example.com".to_string(),
                model: vec!["model".to_string()],
                api_keys: vec!["key-a".to_string(), "key-b".to_string()],
                user_agent_claude: None,
                user_agent_codex: None,
                mode: vec![Mode::OpenAIResponses].into(),
            },
            UpstreamConfig {
                enable: true,
                name: "no-keys".to_string(),
                base_url: "https://empty.example.com".to_string(),
                model: vec!["model".to_string()],
                api_keys: vec![],
                user_agent_claude: None,
                user_agent_codex: None,
                mode: vec![Mode::OpenAIResponses].into(),
            },
        ];
        let selector =
            UpstreamSelector::new(None, upstreams).expect("测试数据已确保 upstreams 非空");

        // 每次调用都推进计数，不论请求成功还是失败重试都轮换到下一个 key
        assert_eq!(selector.next_api_key(0), "key-a");
        assert_eq!(selector.next_api_key(0), "key-b");
        assert_eq!(selector.next_api_key(0), "key-a");
        assert_eq!(selector.next_api_key(0), "key-b");

        // 未配置 key 或下标越界时返回空串
        assert_eq!(selector.next_api_key(1), "");
        assert_eq!(selector.next_api_key(9), "");
    }

    #[test]
    fn test_disabled_upstreams_handling() {
        let upstreams = vec![
            UpstreamConfig {
                enable: false,
                name: "disabled-upstream".to_string(),
                base_url: "https://disabled.example.com".to_string(),
                model: vec!["disabled-model".to_string()],
                api_keys: vec!["disabled-key".to_string()],
                user_agent_claude: None,
                user_agent_codex: None,
                mode: vec![Mode::OpenAIResponses].into(),
            },
            UpstreamConfig {
                enable: true,
                name: "enabled-upstream".to_string(),
                base_url: "https://enabled.example.com".to_string(),
                model: vec!["enabled-model".to_string()],
                api_keys: vec!["enabled-key".to_string()],
                user_agent_claude: None,
                user_agent_codex: None,
                mode: vec![Mode::OpenAIResponses].into(),
            },
        ];
        let selector =
            UpstreamSelector::new(None, upstreams).expect("测试数据已确保 upstreams 非空");

        // 应跳过禁用项
        let (idx, _, _, _, _, _) = selector
            .next_by_mode(Mode::OpenAIResponses)
            .expect("应跳过禁用 upstream");
        assert_eq!(idx, 1);

        // 全部禁用时返回 None
        let all_disabled = vec![UpstreamConfig {
            enable: false,
            name: "disabled".to_string(),
            base_url: "https://disabled.example.com".to_string(),
            model: vec!["model".to_string()],
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
                model: vec!["shared-model".to_string()],
                api_keys: vec!["shared-key-1".to_string(), "shared-key-2".to_string()],
                user_agent_claude: Some("Claude-UA/1.0".to_string()),
                user_agent_codex: Some("Codex-UA/1.0".to_string()),
                mode: vec![Mode::AnthropicDirect, Mode::OpenAIResponses].into(),
            },
            UpstreamConfig {
                enable: true,
                name: "responses-only".to_string(),
                base_url: "https://responses.example.com".to_string(),
                model: vec!["responses-model".to_string()],
                api_keys: vec!["responses-key".to_string()],
                user_agent_claude: None,
                user_agent_codex: None,
                mode: vec![Mode::OpenAIResponses].into(),
            },
        ];
        let selector =
            UpstreamSelector::new(None, upstreams).expect("测试数据已确保 upstreams 非空");

        // 验证多协议 upstream 支持 AnthropicDirect
        let (idx, _, _, _, user_agent, mode) = selector
            .next_by_mode(Mode::AnthropicDirect)
            .expect("多协议 upstream 应支持 anthropic");
        assert_eq!(
            (idx, user_agent, mode),
            (0, Some("Claude-UA/1.0"), Mode::AnthropicDirect)
        );

        // 验证多协议 upstream 也支持 OpenAIResponses
        let (idx, _, _, _, user_agent, mode) = selector
            .next_by_mode(Mode::OpenAIResponses)
            .expect("多协议 upstream 应支持 openai_responses");
        assert_eq!(
            (idx, user_agent, mode),
            (0, Some("Codex-UA/1.0"), Mode::OpenAIResponses)
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
                model: vec!["model-1".to_string()],
                api_keys: vec!["key-1a".to_string(), "key-1b".to_string()],
                user_agent_claude: None,
                user_agent_codex: None,
                mode: vec![Mode::AnthropicDirect].into(),
            },
            UpstreamConfig {
                enable: false,
                name: "forced-upstream".to_string(),
                base_url: "https://forced.example.com".to_string(),
                model: vec!["model-2".to_string()],
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

        // 验证强制索引命中，且 key 在重试尝试间轮换
        let (first, _, _, _, _, _) = selector
            .next_by_mode(Mode::AnthropicDirect)
            .expect("应命中强制索引");
        let (second, _, _, _, _, _) = selector
            .next_by_mode(Mode::AnthropicDirect)
            .expect("应继续命中强制索引");
        let (third, _, _, _, _, _) = selector
            .next_by_mode(Mode::AnthropicDirect)
            .expect("应继续命中强制索引");

        assert_eq!((first, second, third), (1, 1, 1));
        assert_eq!(selector.next_api_key(first), "key-2a");
        assert_eq!(selector.next_api_key(first), "key-2b");
        assert_eq!(selector.next_api_key(first), "key-2a");

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
                model: vec!["model-o".to_string()],
                api_keys: vec!["key-o".to_string()],
                user_agent_claude: None,
                user_agent_codex: None,
                mode: vec![Mode::OpenAIResponses].into(),
            },
            UpstreamConfig {
                enable: true,
                name: "anthropic-upstream".to_string(),
                base_url: "https://anthropic.example.com".to_string(),
                model: vec!["model-a".to_string()],
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
        let (idx, _, _, _, _, _) = selector
            .next_by_mode(Mode::AnthropicDirect)
            .expect("应跳过不支持的 upstream[0]");
        assert_eq!(idx, 1);

        // 验证不支持的 mode 返回 None
        let single_mode_selector = UpstreamSelector::new_with_global_user_agents(
            GlobalUserAgentConfig::default(),
            vec![1],
            vec![
                UpstreamConfig {
                    enable: true,
                    name: "anthropic-only".to_string(),
                    base_url: "https://anthropic.example.com".to_string(),
                    model: vec!["model-a".to_string()],
                    api_keys: vec!["key-a".to_string()],
                    user_agent_claude: None,
                    user_agent_codex: None,
                    mode: vec![Mode::AnthropicDirect].into(),
                },
                UpstreamConfig {
                    enable: false,
                    name: "responses-upstream".to_string(),
                    base_url: "https://responses.example.com".to_string(),
                    model: vec!["model-r".to_string()],
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
    fn test_next_by_mode_and_model_filters_by_request_model() {
        let upstreams = vec![
            UpstreamConfig {
                enable: true,
                name: "model-a-upstream".to_string(),
                base_url: "https://a.example.com".to_string(),
                model: vec!["model-a".to_string()],
                api_keys: vec!["key-a".to_string()],
                user_agent_claude: None,
                user_agent_codex: None,
                mode: vec![Mode::OpenAIResponses].into(),
            },
            UpstreamConfig {
                enable: true,
                name: "model-b-upstream".to_string(),
                base_url: "https://b.example.com".to_string(),
                model: vec!["model-b".to_string()],
                api_keys: vec!["key-b".to_string()],
                user_agent_claude: None,
                user_agent_codex: None,
                mode: vec![Mode::OpenAIResponses].into(),
            },
        ];
        let selector =
            UpstreamSelector::new(None, upstreams).expect("测试数据已确保 upstreams 非空");

        assert_eq!(
            selector.matching_count_by_mode_and_model(Mode::OpenAIResponses, Some("model-b")),
            1
        );
        for _ in 0..4 {
            let (idx, _, _, _, _, _) = selector
                .next_by_mode_and_model(Mode::OpenAIResponses, Some("model-b"))
                .expect("应只命中包含 model-b 的 upstream");
            assert_eq!(idx, 1);
        }
        assert!(
            selector
                .next_by_mode_and_model(Mode::OpenAIResponses, Some("model-x"))
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
                    model: vec!["ua-model".to_string()],
                    api_keys: vec!["ua-key".to_string()],
                    user_agent_claude: case.upstream_claude.map(str::to_owned),
                    user_agent_codex: case.upstream_codex.map(str::to_owned),
                    mode: case.upstream_mode.into(),
                }],
            )
            .expect("测试数据已确保 upstreams 非空");

            let (_, _, _, _, user_agent, mode) = selector
                .next_by_mode(case.mode)
                .expect("应能返回匹配 mode 的 upstream");

            assert_eq!(mode, case.mode, "{}", case.name);
            assert_eq!(user_agent, case.expected_user_agent, "{}", case.name);
        }
    }
}
