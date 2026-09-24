use std::sync::Arc;

use bytes::Bytes;
use http::{HeaderName, HeaderValue};
use http_body_util::Full;
use hyper_rustls::HttpsConnectorBuilder;
use hyper_util::{
    client::legacy::{Client, connect::HttpConnector},
    rt::TokioExecutor,
};
use my_config::{AtomicConfig, Mode};
use salvo::prelude::*;

/// HTTP 客户端类型别名
pub type HttpClient = Client<hyper_rustls::HttpsConnector<HttpConnector>, Full<Bytes>>;

/// 创建支持 HTTP 和 HTTPS 的共享客户端
#[must_use]
pub fn create_http_client() -> Arc<HttpClient> {
    // 使用 webpki-roots 内置证书，不依赖系统证书，提高跨平台稳定性
    let https = HttpsConnectorBuilder::new()
        .with_webpki_roots()
        .https_or_http()
        .enable_http1()
        .build();

    let client = Client::builder(TokioExecutor::new()).build(https);
    Arc::new(client)
}

#[derive(Clone, Copy)]
pub enum ProxyKind {
    Anthropic,
    OpenAI,
}

#[derive(Clone, Copy)]
pub struct ProxyPlan {
    pub(crate) kind: ProxyKind,
    pub(crate) upstream_mode: Mode,
    pub(crate) missing_upstream_message: &'static str,
}

pub struct SelectedUpstream {
    pub(crate) index: usize,
    pub(crate) name: String,
    pub(crate) base_url: String,
    pub(crate) models: Vec<String>,
    pub(crate) api_keys: Vec<String>,
    pub(crate) api_key_index: usize,
    pub(crate) user_agent: Option<String>,
    pub(crate) mode: Mode,
}

impl SelectedUpstream {
    pub(crate) fn display_name(&self) -> &str {
        if self.name.is_empty() {
            "-"
        } else {
            &self.name
        }
    }
}

pub struct FailedUpstreamResponse {
    pub(crate) status: StatusCode,
    pub(crate) headers: Vec<(Option<HeaderName>, HeaderValue)>,
    pub(crate) body: Vec<u8>,
    pub(crate) body_text: String,
}

pub enum UpstreamAttemptFailure {
    Response(FailedUpstreamResponse),
    Transport(String),
}

pub enum RetryLoopResult {
    Forwarded,
    Failed(UpstreamAttemptFailure),
    NoSelection,
}

pub struct RetryContext<'a> {
    pub(crate) req: &'a Request,
    pub(crate) res: &'a mut Response,
    pub(crate) client: &'a Arc<HttpClient>,
    pub(crate) atomic_config: &'a Arc<AtomicConfig>,
    pub(crate) body_bytes: &'a Bytes,
    pub(crate) request_model: Option<&'a str>,
}
