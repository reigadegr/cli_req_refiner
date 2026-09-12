use std::sync::Arc;

use bytes::Bytes;
use http::{Error as HttpError, HeaderValue};
use http_body_util::Full;
use hyper::Request as HyperRequest;
use my_config::Config;
use salvo::prelude::*;

use super::{
    body::{parse_body_json, serialize_body_json},
    intercept::{req_local_intercept_by_url, req_local_intercept_from_json},
    model::strip_billing_header_from_system,
};
use crate::{
    response::log_full_body,
    service::{RequestStats, calculate_tokens, calculate_tokens_from_json},
    types::{ProxyKind, ProxyPlan},
};

pub fn prepare_request_body(
    plan: ProxyPlan,
    body_bytes: Bytes,
    request_url: &str,
    cfg: &Config,
    stats: Option<&Arc<RequestStats>>,
    res: &mut Response,
) -> Option<Bytes> {
    let mut current = body_bytes;
    let mut token_stats_recorded = false;

    if matches!(plan.kind, ProxyKind::Anthropic)
        && req_local_intercept_by_url(res, request_url, cfg)
    {
        return None;
    }

    if matches!(plan.kind, ProxyKind::Anthropic) && !current.is_empty() {
        if let Ok(mut request_json) = parse_body_json(&current) {
            if cfg.optimizations.enable_strip_billing_header {
                strip_billing_header_from_system(&mut request_json);
            }

            if req_local_intercept_from_json(res, &request_json, request_url, cfg) {
                return None;
            }

            if let Some(stats) = stats {
                calculate_tokens_from_json(stats.as_ref(), &request_json);
                token_stats_recorded = true;
            }

            if let Ok(updated) = serialize_body_json(&request_json) {
                current = updated;
            }
        } else {
            tracing::debug!("Skipping Claude body refinement because request JSON parsing failed");
        }
    }

    if !current.is_empty()
        && let Ok(body_str) = std::str::from_utf8(&current)
    {
        if cfg.server.log_req_body {
            log_full_body(body_str);
        }

        if matches!(plan.kind, ProxyKind::Anthropic)
            && !token_stats_recorded
            && let Some(stats) = stats
        {
            calculate_tokens(stats.as_ref(), body_str);
        }
    }

    Some(current)
}

pub fn build_proxy_request(
    req: &Request,
    upstream_url: &str,
    host: &str,
    api_key: &str,
    upstream_user_agent: Option<&str>,
    body_bytes: Bytes,
) -> Result<HyperRequest<Full<Bytes>>, HttpError> {
    let override_user_agent = resolve_upstream_user_agent(upstream_user_agent, host);
    let mut proxy_req_builder = HyperRequest::builder()
        .method(req.method())
        .uri(upstream_url);

    for (name, value) in req.headers() {
        let name_str = name.as_str();
        let should_skip_original_user_agent =
            override_user_agent.is_some() && name == http::header::USER_AGENT;
        if name_str != "host"
            && name_str != "authorization"
            && name_str != "content-length"
            && !should_skip_original_user_agent
        {
            proxy_req_builder = proxy_req_builder.header(name, value);
        }
    }

    proxy_req_builder = proxy_req_builder.header("Authorization", format!("Bearer {api_key}"));
    proxy_req_builder = proxy_req_builder.header("host", host);
    if let Some(user_agent) = override_user_agent {
        proxy_req_builder = proxy_req_builder.header(http::header::USER_AGENT, user_agent);
    }

    proxy_req_builder.body(Full::new(body_bytes))
}

fn resolve_upstream_user_agent(
    upstream_user_agent: Option<&str>,
    host: &str,
) -> Option<HeaderValue> {
    let upstream_user_agent = upstream_user_agent?.trim();
    if upstream_user_agent.is_empty() {
        return None;
    }

    match HeaderValue::from_str(upstream_user_agent) {
        Ok(value) => Some(value),
        Err(error) => {
            tracing::error!(
                "Ignoring invalid upstream user_agent for host {}: {}",
                host,
                error
            );
            None
        }
    }
}

#[cfg(test)]
mod tests {
    use bytes::Bytes;
    use http::uri::Scheme;
    use http_body_util::Full;
    use hyper::Request as HyperRequest;
    use my_config::{Config, Mode, OptimizationConfig, ServerConfig};
    use salvo::{Request, http::StatusCode};

    use super::{build_proxy_request, prepare_request_body};
    use crate::{
        entry::proxy_plan_for_mode,
        response::should_retry_upstream_status,
        types::{ProxyKind, ProxyPlan},
    };

    fn make_request(user_agent: &str) -> Request {
        let req_result = HyperRequest::builder()
            .method("POST")
            .uri("http://localhost/v1/messages")
            .header(http::header::USER_AGENT, user_agent)
            .header("x-test-header", "keep-me")
            .body(Bytes::from_static(br#"{"model":"demo"}"#));
        let Ok(req) = req_result else {
            panic!("failed to build test request");
        };

        Request::from_hyper(req, Scheme::HTTP)
    }

    fn build_proxy_request_for_test(
        req: &Request,
        upstream_user_agent: Option<&str>,
    ) -> HyperRequest<Full<Bytes>> {
        let proxy_req_result = build_proxy_request(
            req,
            "https://upstream.example.com/v1/messages",
            "upstream.example.com",
            "secret",
            upstream_user_agent,
            Bytes::new(),
        );
        let Ok(proxy_req) = proxy_req_result else {
            panic!("failed to build proxy request");
        };
        proxy_req
    }

    fn header_value_as_str(
        request: &HyperRequest<Full<Bytes>>,
        header_name: http::header::HeaderName,
    ) -> Option<&str> {
        request.headers().get(header_name)?.to_str().ok()
    }

    fn named_header_value_as_str<'a>(
        request: &'a HyperRequest<Full<Bytes>>,
        header_name: &str,
    ) -> Option<&'a str> {
        request.headers().get(header_name)?.to_str().ok()
    }

    fn test_config() -> Config {
        Config {
            server: ServerConfig::default(),
            upstream: Vec::new(),
            optimizations: OptimizationConfig::default(),
        }
    }

    fn anthropic_plan() -> ProxyPlan {
        proxy_plan_for_mode(Mode::AnthropicDirect)
    }

    #[test]
    fn proxy_plan_for_mode_maps_supported_modes() {
        let anthropic = proxy_plan_for_mode(Mode::AnthropicDirect);
        assert!(matches!(anthropic.kind, ProxyKind::Anthropic));
        assert_eq!(anthropic.upstream_mode, Mode::AnthropicDirect);
        assert!(anthropic.missing_upstream_message.contains("anthropic"));

        let responses = proxy_plan_for_mode(Mode::OpenAIResponses);
        assert!(matches!(responses.kind, ProxyKind::OpenAI));
        assert_eq!(responses.upstream_mode, Mode::OpenAIResponses);
        assert!(
            responses
                .missing_upstream_message
                .contains("openai_responses")
        );

        let chat = proxy_plan_for_mode(Mode::OpenAIChat);
        assert!(matches!(chat.kind, ProxyKind::OpenAI));
        assert_eq!(chat.upstream_mode, Mode::OpenAIChat);
        assert!(chat.missing_upstream_message.contains("openai_chat"));
    }

    #[test]
    fn should_retry_when_upstream_status_is_not_success() {
        assert!(should_retry_upstream_status(StatusCode::BAD_REQUEST));
        assert!(should_retry_upstream_status(StatusCode::TOO_MANY_REQUESTS));
        assert!(should_retry_upstream_status(
            StatusCode::INTERNAL_SERVER_ERROR
        ));
        assert!(!should_retry_upstream_status(StatusCode::OK));
        assert!(!should_retry_upstream_status(StatusCode::CREATED));
    }

    #[test]
    fn build_proxy_request_resolves_user_agent_override_cases() {
        struct Case {
            name: &'static str,
            upstream_user_agent: Option<&'static str>,
            expected_user_agent: &'static str,
        }

        let cases = [
            Case {
                name: "preserves original when override missing",
                upstream_user_agent: None,
                expected_user_agent: "Original-UA/1.0",
            },
            Case {
                name: "applies configured override",
                upstream_user_agent: Some("Configured-UA/2.0"),
                expected_user_agent: "Configured-UA/2.0",
            },
            Case {
                name: "ignores blank override",
                upstream_user_agent: Some("   "),
                expected_user_agent: "Original-UA/1.0",
            },
            Case {
                name: "ignores invalid override",
                upstream_user_agent: Some("bad\r\nua"),
                expected_user_agent: "Original-UA/1.0",
            },
        ];

        for case in cases {
            let req = make_request("Original-UA/1.0");
            let proxy_req = build_proxy_request_for_test(&req, case.upstream_user_agent);

            assert_eq!(
                header_value_as_str(&proxy_req, http::header::USER_AGENT),
                Some(case.expected_user_agent),
                "{}",
                case.name
            );
            assert_eq!(
                named_header_value_as_str(&proxy_req, "x-test-header"),
                Some("keep-me"),
                "{}",
                case.name
            );
        }
    }

    #[test]
    fn prepare_request_body_intercepts_count_tokens_url_with_empty_body() {
        let mut res = salvo::Response::new();

        let body = prepare_request_body(
            anthropic_plan(),
            Bytes::new(),
            "/v1/messages/count_tokens?foo=bar",
            &test_config(),
            None,
            &mut res,
        );

        assert!(body.is_none());
        assert_eq!(res.status_code, Some(StatusCode::OK));
        assert_eq!(
            res.headers()
                .get("x-cc-proxy-optimization")
                .and_then(|value| value.to_str().ok()),
            Some("max_tokens_mock")
        );
    }
}
