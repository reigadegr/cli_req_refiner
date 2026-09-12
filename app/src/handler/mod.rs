use std::sync::Arc;

use anyhow::{Result, bail};
use my_config::AtomicConfig;
use my_proxy::{
    HttpClient, RequestStats, RouteTarget, classify_request_path,
    handle_anthropic as run_anthropic_proxy, handle_openai as run_openai_proxy,
    rewrite_short_alias,
};
use salvo::prelude::*;

fn setup_handler_state(
    depot: &Depot,
) -> Result<(&Arc<AtomicConfig>, &Arc<RequestStats>, &Arc<HttpClient>)> {
    let Ok(config) = depot.get_typed::<Arc<AtomicConfig>>() else {
        bail!("AtomicConfig not found in depot");
    };
    let Ok(stats) = depot.get_typed::<Arc<RequestStats>>() else {
        bail!("RequestStats not found in depot");
    };
    let Ok(client) = depot.get_typed::<Arc<HttpClient>>() else {
        bail!("HttpClient not found in depot");
    };
    Ok((config, stats, client))
}

async fn dispatch_proxy(req: &mut Request, depot: &Depot, res: &mut Response) {
    let Some(target) = classify_request_path(req.uri().path()) else {
        tracing::info!("Rejecting unsupported proxy path: {}", req.uri().path());
        res.status_code(StatusCode::NOT_FOUND);
        return;
    };

    let (config, stats, client) = match setup_handler_state(depot) {
        Ok(state) => state,
        Err(e) => {
            res.status_code(StatusCode::INTERNAL_SERVER_ERROR);
            tracing::error!("Failed to get dependencies from depot: {e}");
            return;
        }
    };

    match target {
        RouteTarget::Anthropic => run_anthropic_proxy(req, res, config, stats, client).await,
        RouteTarget::OpenAIResponses => {
            run_openai_proxy(req, res, config, client, my_config::Mode::OpenAIResponses).await;
        }
        RouteTarget::OpenAIChat => {
            run_openai_proxy(req, res, config, client, my_config::Mode::OpenAIChat).await;
        }
    }
}

#[endpoint]
pub async fn responses_alias_proxy(req: &mut Request, depot: &mut Depot, res: &mut Response) {
    rewrite_short_alias(req, "/responses", "/v1/responses");
    dispatch_proxy(req, depot, res).await;
}

#[endpoint]
pub async fn chat_completions_alias_proxy(
    req: &mut Request,
    depot: &mut Depot,
    res: &mut Response,
) {
    rewrite_short_alias(req, "/chat/completions", "/v1/chat/completions");
    dispatch_proxy(req, depot, res).await;
}

#[endpoint]
pub async fn unified_proxy(req: &mut Request, depot: &mut Depot, res: &mut Response) {
    dispatch_proxy(req, depot, res).await;
}
