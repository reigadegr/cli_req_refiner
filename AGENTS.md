## 项目概述

**CliReqRefiner** 是面向 AI 编程工具（Claude Code、Codex 等）的高性能 API 代理网关，核心功能是请求体精炼优化。

### 核心特性
- **多上游负载均衡** - 双层轮询策略（先选上游，再轮询 API key）
- **热配置重载** - 通过 `notify` crate 监听 `config.toml`，修改后立即生效
- **本地优化拦截** - 减少消耗：
  - 配额/网络探测检查
  - 标题生成
  - 建议模式
  - 历史分析
  - 文件路径提取
  - 计费头清理（可选）
- **请求统计** - 追踪 Token 消耗（总量、用户输入、历史上下文、助手回复、系统提示）

## 常用命令

### 构建
```bash
# Debug 模式
sh build_native_stable.sh

# Release 模式（推荐用于生产环境）
sh build_native_stable.sh r
```

> ⚠️ 全程禁止执行 `cargo build`、`cargo build --release` 以及其他任何 `cargo build*` 构建命令。
> 如需验证或测试，请统一执行 `sh debug.sh`（已包含格式化、clippy 与 `cargo test` 的全套检查）。

### 运行
通知用户手动运行

### 开发调试
```bash
sh debug.sh
```

## 提交说明

提交时必须遵循约定式提交（Conventional Commits）。

提交标题必须完整，并且标题使用中文。

提交正文的第一行必须是提交标题的英文翻译。

提交正文的主要内容必须采用中英逐句对照的写法，先中文，下一行对应英文，逐句成对出现。

提交信息必须包含 DCO。

DCO 中的姓名和邮箱必须从本地 git 配置获取，不得手写、不得使用占位符、不得替换为其他身份。

获取 DCO 身份时，使用以下命令读取本地配置：

```bash
git config user.name
git config user.email
```

生成 `Signed-off-by` 时，必须直接使用上面两个命令的输出结果。

## 补丁说明

如果补丁工具提示打补丁失败，这有可能是误报。

遇到这种情况时，请先执行 `git diff` 检查刚才的修改是否已经正确应用，因为大部分这类报错都是误报。

在确认 `git diff` 后，再重新读取修改后的文件内容，判断补丁是否真的失败；不要仅凭补丁工具的返回结果下结论。

## 架构概览

项目采用 workspace 结构，自底向上依赖：`selector` → `config` → `proxy` → `app`。

### 入口 (`app/src/`)
- **`main.rs`** - 初始化日志、`AtomicConfig`、启动文件监听器，构建 Salvo 路由并按配置端口（默认 `0.0.0.0:9077`）启动服务器
- **`gateway.rs`** - `GatewayHandler` 持有共享的 `HttpClient`（hyper + HTTPS）和 `RequestStats`
- **`handler/mod.rs`** - 3 个 Salvo endpoint：`unified_proxy`、`responses_alias_proxy`、`chat_completions_alias_proxy`，根据 `classify_request_path` 分发到不同协议处理器

### 上游选择器 (`crates/selector/src/`)
- **`lib.rs`** - 模块声明与导出
- **`model.rs`** - `UpstreamConfig`、`Mode`（`AnthropicDirect` / `OpenAIResponses` / `OpenAIChat`）、`UpstreamModes`、`GlobalUserAgentConfig` 定义
- **`selector.rs`** - `UpstreamSelector` 实现双层轮询：先选上游，再轮询其 API keys；支持 `force_upstream_index` 强制轮询范围

### 配置系统 (`crates/config/src/`)
- **`lib.rs`** - 模块声明，对外导出 `AtomicConfig`、`Config`、`OptimizationConfig`、`ServerConfig` 以及 selector 相关类型
- **`model.rs`** - `Config`、`ServerConfig`、`OptimizationConfig` 定义；支持旧版顶层字段向后兼容
- **`runtime.rs`** - `AtomicConfig` 使用 `arc-swap` 实现无锁热重载；持有 `ArcSwap<Config>` 和 `ArcSwap<Option<Arc<UpstreamSelector>>>`
- **`loader.rs`** - 配置文件加载、格式化、写回
- **`watcher.rs`** - `notify` crate 文件监听，`CloseWrite` 事件触发重载
- **`format.rs`** - taplo TOML 格式化工具

### 代理核心 (`crates/proxy/src/`)
- **`lib.rs`** - 模块声明与所有公开导出
- **`entry.rs`** - `handle_anthropic` / `handle_openai` 入口函数，含重试循环（最多 30 次）与退避策略
- **`routing.rs`** - `RouteTarget` 枚举、`classify_request_path` 路由分类、`make_proxy_url` 代理 URL 构建、`rewrite_short_alias` 短路径重写
- **`service.rs`** - `RequestStats` 原子计数器（总 token、用户输入、历史上下文、助手回复、系统提示）、`calculate_tokens` 统计函数
- **`types.rs`** - `HttpClient` 类型别名、`create_http_client()`、`ProxyPlan`、`SelectedUpstream`、重试相关类型

#### 请求处理 (`crates/proxy/src/request/`)
- **`body.rs`** - `get_req_body` / `parse_body_json` / `serialize_body_json`
- **`build.rs`** - `prepare_request_body`（请求体准备全流程）、`build_proxy_request`（构建代理请求）
- **`intercept.rs`** - 调用优化模块进行本地拦截
- **`model.rs`** - `override_model_in_json` / `strip_billing_header_from_system`

#### 优化层 (`crates/proxy/src/request/optimization/`)
- **`detection.rs`** - 请求类型检测（配额检查、标题生成等）
- **`rules.rs`** - `OptimizationRuleMatch` 枚举与规则匹配逻辑
- **`engine.rs`** - 优化引擎：规则匹配 → mock 响应构建
- **`response_builder.rs`** - `OptimizationResponse` 构建器
- **`command_utils.rs`** - 命令前缀提取工具

#### 响应处理 (`crates/proxy/src/response/`)
- **`forward.rs`** - SSE 流式透传 + 非流式收集转发
- **`failed.rs`** - 上游失败响应收集与渲染
- **`decompress.rs`** - gzip 解压
- **`logging.rs`** - 请求/响应体日志

## 配置说明

代理读取 `config.toml`（或第一个命令行参数指定的路径）。示例：

```toml
[server]
port = 9077
force_upstream_index = []
log_req_body = false
log_res_body = false
user_agent_global_claude = "Claude-Code/1.0.84 (Linux; Android 14)"
user_agent_global_codex = "Codex/0.31.0 (Linux; Android 14)"

# 上游 1
[[upstream]]
enable = true
name = "zhipu-main"
base_url = "https://open.bigmodel.cn/api/anthropic"
model = "glm-4.7"
api_keys = ["your_api_key1", "your_api_key2"]
user_agent_claude = "Claude-Code/1.0.84 (Linux; Android 14)"
user_agent_codex = "Codex/0.31.0 (Linux; Android 14)"
# mode 默认为 "anthropic"，也支持数组，例如 ["anthropic", "openai_responses"]
# 设置 enable = false 可临时禁用该 upstream

[optimizations]
enable_network_probe_mock = true
enable_fast_prefix_detection = true
enable_historical_analysis_mock = true
enable_title_generation_skip = true
enable_suggestion_mode_skip = true
enable_filepath_extraction_mock = true
enable_strip_billing_header = true
```

配置变更会通过 `notify` crate 自动检测并重载，无需重启服务。
监听端口 `server.port` 仅在启动时读取，修改后需要重启服务。
旧版顶层 `port`、`log_req_body`、`log_res_body`、`user_agent_global_*` 仍兼容读取，推荐逐步迁移到 `[server]` 配置块。

## 请求流程

1. 客户端（Claude Code / Codex）向代理发送请求
2. 路由层通过短路径别名（`/responses` → `/v1/responses`，`/chat/completions` → `/v1/chat/completions`）或直接 `/v1/**` 进入 `unified_proxy`
3. `classify_request_path` 根据路径分发：`/v1/messages` → Anthropic，`/v1/responses` → OpenAI Responses，`/v1/chat/completions` → OpenAI Chat
4. `prepare_request_body` 执行请求体准备：URL 拦截检测（`count_tokens`）→ 可选的 `strip_billing_header_from_system`（受 `enable_strip_billing_header` 控制）→ JSON 拦截检测（配额/标题/建议/历史/文件路径）→ Token 统计
5. 如需拦截 → `OptimizationResponse` 返回本地 mock 响应
6. 否则 → `try_upstreams` 重试循环：`select_upstream` 双层轮询 → `apply_upstream_model` → `make_proxy_url` → `build_proxy_request` → 发送请求
7. `forward_proxy_response` 流式返回响应（SSE 透传或非流式收集 + gzip 解压），失败则退避重试下一个上游

## 核心技术栈

- **[Salvo](https://salvo.rs/)** - 异步 Web 框架
- **[Hyper](https://hyper.rs/)** - HTTP 客户端（支持 HTTP/1.1 & HTTP/2）
- **[hyper-rustls](https://docs.rs/hyper-rustls/)** - TLS 支持（webpki-roots）
- **[Tokio](https://tokio.rs/)** - Rust 异步运行时
- **[arc-swap](https://docs.rs/arc-swap/)** - 无锁原子配置切换
- **[notify](https://docs.rs/notify/)** - 跨平台文件监听
- **[mimalloc](https://github.com/microsoft/mimalloc)** - 高性能内存分配器

## 规范说明

## 1. Think Before Coding

**Don't assume. Don't hide confusion. Surface tradeoffs.**

Before implementing:
- State your assumptions explicitly. If uncertain, ask.
- If multiple interpretations exist, present them - don't pick silently.
- If a simpler approach exists, say so. Push back when warranted.
- If something is unclear, stop. Name what's confusing. Ask.

## 2. Simplicity First

**Minimum code that solves the problem. Nothing speculative.**

- No features beyond what was asked.
- No abstractions for single-use code.
- No "flexibility" or "configurability" that wasn't requested.
- No error handling for impossible scenarios.
- If you write 200 lines and it could be 50, rewrite it.

Ask yourself: "Would a senior engineer say this is overcomplicated?" If yes, simplify.

## 3. Surgical Changes

**Touch only what you must. Clean up only your own mess.**

When editing existing code:
- Don't "improve" adjacent code, comments, or formatting.
- Don't refactor things that aren't broken.
- Match existing style, even if you'd do it differently.
- If you notice unrelated dead code, mention it - don't delete it.

When your changes create orphans:
- Remove imports/variables/functions that YOUR changes made unused.
- Don't remove pre-existing dead code unless asked.

The test: Every changed line should trace directly to the user's request.

## 4. Goal-Driven Execution

**Define success criteria. Loop until verified.**

Transform tasks into verifiable goals:
- "Add validation" → "Write tests for invalid inputs, then make them pass"
- "Fix the bug" → "Write a test that reproduces it, then make it pass"
- "Refactor X" → "Ensure tests pass before and after"

For multi-step tasks, state a brief plan:
```
1. [Step] → verify: [check]
2. [Step] → verify: [check]
3. [Step] → verify: [check]
```

Strong success criteria let you loop independently. Weak criteria ("make it work") require constant clarification.

---

**These guidelines are working if:** fewer unnecessary changes in diffs, fewer rewrites due to overcomplication, and clarifying questions come before implementation rather than after mistakes.
