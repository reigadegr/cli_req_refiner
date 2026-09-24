use std::{sync::Arc, time::Duration};

use bytes::Bytes;
use my_config::{AtomicConfig, Mode, UpstreamSelector};
use salvo::prelude::*;
use serde_json::from_slice;

use super::{
    request::prepare_request_body,
    response::{forward_proxy_response, render_failed_upstream_response},
    service::RequestStats,
    types::{
        FailedUpstreamResponse, HttpClient, ProxyKind, ProxyPlan, RetryContext, RetryLoopResult,
        SelectedUpstream, UpstreamAttemptFailure,
    },
};
use crate::{
    request::{get_req_body, override_model_in_body},
    response::log_request_meta,
    routing::make_proxy_url,
};

const MAX_UPSTREAM_ATTEMPTS: usize = 300;

/// 指数退避休眠时长（毫秒），按重试次数索引，最长封顶 1s
const RETRY_DELAYS_MS: [u64; 5] = [300, 400, 600, 800, 1000];

fn retry_delay_ms(retry_count: usize) -> u64 {
    let index = retry_count.saturating_sub(1).min(RETRY_DELAYS_MS.len() - 1);
    RETRY_DELAYS_MS[index]
}

/// 从请求体中提取 model 字段
fn extract_model_from_body(body_bytes: &[u8]) -> Option<String> {
    if body_bytes.is_empty() {
        return None;
    }

    let json: serde_json::Value = from_slice(body_bytes).ok()?;
    json.get("model")?.as_str().map(str::to_owned)
}

pub const fn proxy_plan_for_mode(mode: Mode) -> ProxyPlan {
    match mode {
        Mode::AnthropicDirect => ProxyPlan {
            kind: ProxyKind::Anthropic,
            upstream_mode: Mode::AnthropicDirect,
            missing_upstream_message: "No upstream configured with mode including 'anthropic'",
        },
        Mode::OpenAIResponses => ProxyPlan {
            kind: ProxyKind::OpenAI,
            upstream_mode: Mode::OpenAIResponses,
            missing_upstream_message: "No upstream configured with mode including 'openai_responses'",
        },
        Mode::OpenAIChat => ProxyPlan {
            kind: ProxyKind::OpenAI,
            upstream_mode: Mode::OpenAIChat,
            missing_upstream_message: "No upstream configured with mode including 'openai_chat'",
        },
    }
}

pub async fn handle_anthropic(
    req: &mut Request,
    res: &mut Response,
    config: &Arc<AtomicConfig>,
    stats: &Arc<RequestStats>,
    client: &Arc<HttpClient>,
) {
    run_proxy(
        proxy_plan_for_mode(Mode::AnthropicDirect),
        req,
        res,
        config,
        Some(stats),
        client,
    )
    .await;
}

pub async fn handle_openai(
    req: &mut Request,
    res: &mut Response,
    config: &Arc<AtomicConfig>,
    client: &Arc<HttpClient>,
    mode: Mode,
) {
    run_proxy(proxy_plan_for_mode(mode), req, res, config, None, client).await;
}

async fn run_proxy(
    plan: ProxyPlan,
    req: &mut Request,
    res: &mut Response,
    config: &Arc<AtomicConfig>,
    stats: Option<&Arc<RequestStats>>,
    client: &Arc<HttpClient>,
) {
    let body_bytes = match get_req_body(req).await {
        Ok(body) => body,
        Err(error) => {
            tracing::error!("{error}");
            res.status_code(StatusCode::INTERNAL_SERVER_ERROR);
            return;
        }
    };

    let cfg = config.get();
    let request_url = req.uri().to_string();

    log_request_meta(req.method().as_str(), &request_url, req.headers());

    // 提取请求体中的 model 字段，用于过滤 upstream
    let request_model = extract_model_from_body(&body_bytes);
    if let Some(ref model) = request_model {
        tracing::info!("📝 请求体中的 model: {}", model);
    }

    let body_bytes = prepare_request_body(plan, body_bytes, &request_url, &cfg, stats, res);
    let Some(body_bytes) = body_bytes else {
        return;
    };

    let Some(selector) = config.get_upstream_selector() else {
        tracing::error!("{}", plan.missing_upstream_message);
        res.status_code(StatusCode::INTERNAL_SERVER_ERROR);
        return;
    };

    let matching_count =
        selector.matching_count_by_mode_and_model(plan.upstream_mode, request_model.as_deref());
    if matching_count == 0 {
        if let Some(ref model) = request_model {
            tracing::error!(
                "{}: 没有配置 model={} 的 upstream",
                plan.missing_upstream_message,
                model
            );
        } else {
            tracing::error!("{}", plan.missing_upstream_message);
        }
        res.status_code(StatusCode::INTERNAL_SERVER_ERROR);
        return;
    }

    match try_upstreams(
        plan,
        RetryContext {
            req,
            res,
            client,
            atomic_config: config,
            body_bytes: &body_bytes,
            request_model: request_model.as_deref(),
        },
    )
    .await
    {
        RetryLoopResult::Forwarded => {}
        RetryLoopResult::Failed(UpstreamAttemptFailure::Response(failed_response)) => {
            tracing::error!(
                "{} after exhausting model fallback chain; returning last upstream response",
                proxy_failure_label(plan.kind),
            );
            render_failed_upstream_response(res, failed_response);
        }
        RetryLoopResult::Failed(UpstreamAttemptFailure::Transport(error_message)) => {
            tracing::error!(
                "{} after exhausting model fallback chain: {}",
                proxy_failure_label(plan.kind),
                error_message
            );
            res.status_code(StatusCode::BAD_GATEWAY);
            res.render("Bad Gateway");
        }
        RetryLoopResult::NoSelection => {
            tracing::error!(
                "{}: selector returned no upstream during retry loop",
                proxy_failure_label(plan.kind)
            );
            res.status_code(StatusCode::BAD_GATEWAY);
            res.render("Bad Gateway");
        }
    }
}

#[allow(clippy::too_many_lines)]
async fn try_upstreams(plan: ProxyPlan, ctx: RetryContext<'_>) -> RetryLoopResult {
    let mut last_failure = None;

    let current_cfg = ctx.atomic_config.get();
    let forced = !current_cfg.server.force_upstream_index.is_empty();
    let Some(selector) = ctx.atomic_config.get_upstream_selector() else {
        return RetryLoopResult::NoSelection;
    };
    let Some(selected_upstream) = select_upstream(&selector, plan, ctx.request_model) else {
        return RetryLoopResult::NoSelection;
    };

    let models = &selected_upstream.models;
    let max_attempts = if models.is_empty() {
        1
    } else if forced {
        MAX_UPSTREAM_ATTEMPTS
    } else {
        models.len()
    };

    for attempt in 1..=max_attempts {
        if attempt > 1 {
            let retry_count = attempt - 1;
            let delay_ms = retry_delay_ms(retry_count);
            tracing::info!(
                "{}: 第 {} 次重试，休眠 {}ms",
                proxy_failure_label(plan.kind),
                retry_count,
                delay_ms
            );
            tokio::time::sleep(Duration::from_millis(delay_ms)).await;
        }

        let model = if models.is_empty() {
            ""
        } else {
            let model_index = (attempt - 1) % models.len();
            &models[model_index]
        };

        let api_key = selector.next_api_key(selected_upstream.index);

        log_selected_upstream(
            plan.kind,
            &selected_upstream,
            model,
            api_key,
            attempt,
            max_attempts,
        );

        let attempt_body = apply_upstream_model(ctx.body_bytes.clone(), model);
        let (upstream_url, host) = make_proxy_url(&selected_upstream.base_url, ctx.req);

        let proxy_req = match super::request::build_proxy_request(
            ctx.req,
            &upstream_url,
            host,
            api_key,
            selected_upstream.user_agent.as_deref(),
            attempt_body,
        ) {
            Ok(request) => request,
            Err(error) => {
                tracing::error!("Failed to build proxy request: {}", error);
                ctx.res.status_code(StatusCode::INTERNAL_SERVER_ERROR);
                return RetryLoopResult::Forwarded;
            }
        };

        match ctx.client.request(proxy_req).await {
            Ok(proxy_resp) => {
                match forward_proxy_response(plan.kind, proxy_resp, ctx.res, &current_cfg).await {
                    Ok(()) => return RetryLoopResult::Forwarded,
                    Err(UpstreamAttemptFailure::Response(failed_response)) => {
                        log_failed_upstream_response(
                            plan.kind,
                            &selected_upstream,
                            model,
                            attempt,
                            max_attempts,
                            current_cfg.server.log_res_body,
                            &failed_response,
                            forced,
                        );
                        last_failure = Some(UpstreamAttemptFailure::Response(failed_response));
                    }
                    Err(UpstreamAttemptFailure::Transport(error_message)) => {
                        log_transport_failure(
                            plan.kind,
                            &selected_upstream,
                            model,
                            attempt,
                            max_attempts,
                            &error_message,
                            forced,
                        );
                        last_failure = Some(UpstreamAttemptFailure::Transport(error_message));
                    }
                }
            }
            Err(error) => {
                let error_message = error.to_string();
                log_transport_failure(
                    plan.kind,
                    &selected_upstream,
                    model,
                    attempt,
                    max_attempts,
                    &error_message,
                    forced,
                );
                last_failure = Some(UpstreamAttemptFailure::Transport(error_message));
            }
        }
    }

    last_failure.map_or_else(|| RetryLoopResult::NoSelection, RetryLoopResult::Failed)
}

fn select_upstream(
    selector: &UpstreamSelector,
    plan: ProxyPlan,
    request_model: Option<&str>,
) -> Option<SelectedUpstream> {
    let (index, name, base_url, models, user_agent, mode) =
        selector.next_by_mode_and_model(plan.upstream_mode, request_model)?;

    Some(SelectedUpstream {
        index,
        name: name.to_owned(),
        base_url: base_url.to_owned(),
        models: models.to_vec(),
        user_agent: user_agent.map(str::to_owned),
        mode,
    })
}

fn apply_upstream_model(body_bytes: Bytes, model: &str) -> Bytes {
    if model.is_empty() || body_bytes.is_empty() {
        return body_bytes;
    }

    override_model_in_body(&body_bytes, model).unwrap_or(body_bytes)
}

pub const fn proxy_failure_label(kind: ProxyKind) -> &'static str {
    match kind {
        ProxyKind::Anthropic => "Proxy request failed",
        ProxyKind::OpenAI => "OpenAI proxy request failed",
    }
}

fn log_selected_upstream(
    kind: ProxyKind,
    upstream: &SelectedUpstream,
    model: &str,
    api_key: &str,
    attempt: usize,
    total_attempts: usize,
) {
    let prefix = match kind {
        ProxyKind::Anthropic => "🔄 选中的",
        ProxyKind::OpenAI => "🔄 OpenAI 代理选中的",
    };

    tracing::info!(
        "{} Upstream[{}] name={} (attempt {}/{}): base_url={}, model={}, api_key: {}***, mode={:?}",
        prefix,
        upstream.index,
        upstream.display_name(),
        attempt,
        total_attempts,
        upstream.base_url,
        model,
        api_key.chars().take(8).collect::<String>(),
        upstream.mode
    );
}

#[allow(clippy::too_many_arguments)]
fn log_failed_upstream_response(
    kind: ProxyKind,
    upstream: &SelectedUpstream,
    model: &str,
    attempt: usize,
    total_attempts: usize,
    log_response_body: bool,
    failed_response: &FailedUpstreamResponse,
    forced: bool,
) {
    let body_suffix = if log_response_body {
        let body = if failed_response.body_text.is_empty() {
            "<empty body>"
        } else {
            failed_response.body_text.as_str()
        };
        format!(", body={body}")
    } else {
        String::new()
    };

    let name = upstream.display_name();

    if attempt < total_attempts {
        let retry_hint = if forced {
            "重试"
        } else {
            "重试下一个 model"
        };
        tracing::warn!(
            "{}: upstream[{}] name={name} attempt {attempt}/{total_attempts} returned status {}, {retry_hint}; base_url={}, model={}{body_suffix}",
            proxy_failure_label(kind),
            upstream.index,
            failed_response.status,
            upstream.base_url,
            model,
        );
    } else {
        tracing::error!(
            "{}: upstream[{}] name={name} attempt {attempt}/{total_attempts} returned status {}, no model left; base_url={}, model={}{body_suffix}",
            proxy_failure_label(kind),
            upstream.index,
            failed_response.status,
            upstream.base_url,
            model,
        );
    }
}

fn log_transport_failure(
    kind: ProxyKind,
    upstream: &SelectedUpstream,
    model: &str,
    attempt: usize,
    total_attempts: usize,
    error_message: &str,
    forced: bool,
) {
    let name = upstream.display_name();

    if attempt < total_attempts {
        let retry_hint = if forced {
            "重试"
        } else {
            "重试下一个 model"
        };
        tracing::warn!(
            "{}: upstream[{}] name={name} attempt {attempt}/{total_attempts} transport error, {retry_hint}; base_url={}, model={}, error={error_message}",
            proxy_failure_label(kind),
            upstream.index,
            upstream.base_url,
            model,
        );
    } else {
        tracing::error!(
            "{}: upstream[{}] name={name} attempt {attempt}/{total_attempts} transport error, no model left; base_url={}, model={}, error={error_message}",
            proxy_failure_label(kind),
            upstream.index,
            upstream.base_url,
            model,
        );
    }
}
