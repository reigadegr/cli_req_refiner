use std::{path::Path, sync::Arc, time::Duration};

use arc_swap::{ArcSwap, Guard};
use tracing::{error, info};

use crate::{
    Config, UpstreamSelector, enabled_upstream_count,
    loader::{load_from_file, load_initial_config, resolve_config_path},
    watcher::start_config_watcher,
};

/// 全局原子配置，支持热重载
pub struct AtomicConfig {
    inner: ArcSwap<Config>,
    config_path: std::path::PathBuf,
    /// Upstream 选择器（双层轮询：先 upstream，后 `api_keys`）
    upstream_selector: ArcSwap<Option<Arc<UpstreamSelector>>>,
}

impl AtomicConfig {
    /// 初始化配置，从指定路径或默认路径加载
    #[must_use]
    pub fn init() -> Self {
        let config_path = resolve_config_path();
        let config = load_initial_config(&config_path);
        let upstream_selector = UpstreamSelector::new_with_global_user_agents(
            config.global_user_agent_config(),
            config.server.force_upstream_index.clone(),
            config.upstream.clone(),
        )
        .map(Arc::new);

        Self {
            inner: ArcSwap::from(Arc::new(config)),
            config_path,
            upstream_selector: ArcSwap::from(Arc::new(upstream_selector)),
        }
    }

    /// 从文件加载配置
    pub fn get(&self) -> Guard<Arc<Config>> {
        self.inner.load()
    }

    /// 获取 Upstream 选择器（双层轮询）
    pub fn get_upstream_selector(&self) -> Option<Arc<UpstreamSelector>> {
        (**self.upstream_selector.load()).clone()
    }

    /// 重新加载配置
    pub fn reload(&self) {
        std::thread::sleep(Duration::from_millis(50));
        info!("🔄 检测到配置文件变更，正在重新加载...");

        match load_from_file(&self.config_path) {
            Ok(new_config) => {
                let new_global_user_agents = new_config.global_user_agent_config();
                let any_agent_configured = new_global_user_agents.is_any_configured();
                self.inner.store(Arc::new(new_config.clone()));

                let new_selector = UpstreamSelector::new_with_global_user_agents(
                    new_global_user_agents,
                    new_config.server.force_upstream_index.clone(),
                    new_config.upstream.clone(),
                )
                .map(Arc::new);
                self.upstream_selector.store(Arc::new(new_selector));

                info!("✅ 配置已更新");
                info!(
                    "📋 当前配置: upstream={} 个（启用 {} 个）, force_upstream_index={:?}, global_user_agent_configured={}",
                    new_config.upstream.len(),
                    enabled_upstream_count(&new_config.upstream),
                    new_config.server.force_upstream_index,
                    any_agent_configured
                );
            }
            Err(error) => {
                error!("❌ 配置重载失败: {}", error);
            }
        }
    }

    /// 启动配置文件监听（跨平台）
    pub fn start_watcher(self: Arc<Self>) {
        start_config_watcher(self);
    }

    pub(crate) fn config_path(&self) -> &Path {
        &self.config_path
    }
}
