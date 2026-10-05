# ✂️ CliReqRefiner

[![LINUX DO 社区](https://img.shields.io/badge/首发于-LINUX%20DO%20社区-000000?logo=linux&logoColor=white)](https://linux.do/)

<div align="center">

**高性能 AI 编码工具请求体精炼代理**

> ✅ **协议支持**：当前通过统一根路径同时支持 **Claude Code**、**OpenAI Responses / Codex** 与 **OpenAI Chat Completions** 请求路由。

请求体精炼 · 多上游负载均衡 · 热重载配置 · Token 成本削减

[![Rust](https://img.shields.io/badge/rust-1.85+-orange.svg)](https://www.rust-lang.org)
[![License](https://img.shields.io/badge/license-GPLv3-blue.svg)](LICENSE)

[![跨平台](https://img.shields.io/badge/platform-Windows%20%7C%20Linux%20%7C%20macOS-lightgrey.svg)](https://github.com/rust-lang/rust)

</div>

---

## 📖 简介

**CliReqRefiner** 是一款专为 AI 编码工具（Claude Code、Codex 等）设计的高性能 API 代理，核心聚焦于**请求体精炼**以降低 token 消耗。

它可以帮助你：
- ✂️ **精炼请求体** — 移除冗余系统提示词、裁剪工具定义、优化上下文结构
- 🌐 **对接多个上游服务商**，自动负载均衡
- 💰 **降低 API 成本** — 智能拦截非必要请求 + 请求体优化
- ⚡ **加速响应** — 本地处理部分优化请求
- 🔧 **零停机配置** — 修改配置即时生效

### 💡 为什么需要它？

AI 编码工具（Claude Code、Codex 等）在使用过程中会发送大量"探测性"请求（如配额检查、标题生成、建议模式等），这些请求消耗 token 却对实际开发帮助甚微。CliReqRefiner 能够智能识别并拦截这些请求，直接返回本地模拟响应——在保持工具完整功能的同时，大幅降低 token 消耗。

此外，CliReqRefiner 还会**精炼系统提示词和工具定义**，进一步降低 token 用量。实际使用中，AI 编码工具发送的请求往往包含大量预设系统提示词和工具定义，它们在每次请求中重复发送，消耗大量 token。CliReqRefiner 可以：
- 📝 **移除冗余系统提示词**，仅保留核心指令
- 🔧 **裁剪工具定义**，仅保留关键工具描述
- 📊 **优化上下文结构**，减少重复信息

这种双重优化策略，使 CliReqRefiner 能在保持完整功能的前提下，将 API 调用成本降至最低。

---

## ✨ 功能特性

### ✂️ 请求体精炼（核心）

CliReqRefiner 的核心功能——在转发到上游之前精炼请求体：
- 📝 **系统提示词优化** — 移除冗余系统提示词，仅保留核心指令
- 🔧 **工具定义裁剪** — 精简冗长的工具描述
- 📊 **上下文结构优化** — 减少请求间的重复信息
- 🎯 **细粒度控制** — 每项优化可独立启用/禁用

### 🔥 热重载配置

- 配置文件变更时**自动热重载**，无需重启
- 通过 `notify` crate 实现跨平台文件监听
- 平滑切换配置，服务不中断

### ⚡ 本地优化拦截

智能识别特定请求并在本地处理，减少上游调用：

| 优化项 | 说明 |
|:-------|:-----|
| 🔍 **配额检查拦截** | 对配额探测请求返回本地模拟响应 |
| 📝 **快速前缀检测** | 识别并提取命令前缀（如 `git commit`） |
| 📋 **标题生成跳过** | 对标题生成请求返回默认响应 |
| 💡 **建议模式跳过** | 对建议模式请求返回空响应 |
| 📂 **文件路径提取** | 从命令输出中提取文件路径 |
| 📊 **历史分析跳过** | 对历史分析请求返回简化响应 |

### 🔄 多上游负载均衡

- 支持多个上游服务商
- **双层轮询**：上游之间轮询，每个上游内的 API Key 之间也轮询
- API Key 自动轮换，最大化请求分发

### 📊 请求统计与监控

- 实时请求计数和 token 消耗统计
- 区分用户输入 token、上下文 token 和助手响应 token

---

## 🚀 快速开始

### 🎯 配置客户端

将客户端基地址直接指向代理服务根路径：

```bash
# 方式一：环境变量
export ANTHROPIC_BASE_URL="http://127.0.0.1:9077"
```

或在 `~/.claude/settings.json` 中：

```json
{
  "env": {
    "ANTHROPIC_BASE_URL": "http://127.0.0.1:9077",
    "ANTHROPIC_AUTH_TOKEN": "随便填"
  }
}
```

路径映射规则：
- `/v1/messages` 与 `/v1/messages/*` -> Anthropic 协议
- `/v1/responses` -> OpenAI Responses 协议
- `/v1/chat/completions` -> OpenAI Chat Completions 协议
- 其他根路径会直接返回 `404`，不转发到上游

其中：
- `ANTHROPIC_BASE_URL` 指向服务根地址 `http://127.0.0.1:9077`，不要再追加 `/claude` 或 `/codex` 前缀
- `ANTHROPIC_AUTH_TOKEN` 可以随意填写——CliReqRefiner 转发时会覆盖它
- 如果你修改了 `config.toml` 里的 `port`，这里也要同步改成对应端口

### 🧭 统一路径路由

客户端统一指向代理服务根地址，不再需要额外拼接 `/claude` 或 `/codex` 前缀。服务会根据请求路径自动选择协议与上游：

| 请求路径 | 路由协议 | 说明 |
|:---------|:---------|:-----|
| `/v1/messages` | Anthropic | Claude Messages 主入口 |
| `/v1/messages/*` | Anthropic | 保留 Anthropic 子路径，例如 `/v1/messages/count_tokens` |
| `/v1/responses` | OpenAI Responses | Codex / Responses 请求 |
| `/v1/chat/completions` | OpenAI Chat | Chat Completions 请求 |
| 其他路径 | `404 Not Found` | 不会转发到上游 |

示例：

```text
Claude Code              -> http://127.0.0.1:9077/v1/messages
Codex / Responses client -> http://127.0.0.1:9077/v1/responses
OpenAI Chat client       -> http://127.0.0.1:9077/v1/chat/completions
```

> 注意：代理只做**严格路径分流**与请求体精炼，不会在 Anthropic / OpenAI 协议之间做自动转换或回退。

### 📦 构建

```bash
# Debug 模式
sh build_native_stable.sh

# Release 模式（推荐生产使用）
sh build_native_stable.sh r
```

### ⚙️ 配置

编辑 `config.toml`：

```toml
# 负载均衡配置示例
# 可配置多个上游，每个上游支持多个 API Key
# 策略：上游之间轮询，API Key 之间轮询
# 修改后即时生效

[server]
# 服务监听端口（默认 9077，修改后需重启服务）
port = 9077
# 强制轮询的 upstream 下标列表；非空时忽略 enable 字段，仅在列表内轮询；空数组按默认规则轮询
force_upstream_index = []
# 每次上游尝试更换 X-Forwarded-For；仅对信任此请求头的自有上游 IP 限流有效
bypass_ip_rate_limit = false

# 是否打印请求体
log_req_body = false
# 是否打印响应体
log_res_body = false
user_agent_global_claude = "Claude-Code/1.0.84 (Linux; Android 14)"
user_agent_global_codex = "Codex/0.31.0 (Linux; Android 14)" # 对 openai_responses / openai_chat 均生效

# 上游 1
[[upstream]]
enable = true
name = "zhipu-main"
base_url = "https://open.bigmodel.cn/api/anthropic"
model = "glm-4.7"
api_keys = ["your_api_key1", "your_api_key2"]
user_agent_claude = "Claude-Code/1.0.84 (Linux; Android 14)"
user_agent_codex = "Codex/0.31.0 (Linux; Android 14)" # 对 openai_responses / openai_chat 均生效
# mode 默认为 "anthropic"，也支持数组，例如 ["anthropic", "openai_responses"]
# 设置 enable = false 可临时禁用该上游
# 如需同时兼容多种协议，可设置 mode = ["anthropic", "openai_responses"] 或 ["anthropic", "openai_responses", "openai_chat"]

# 上游 2：添加更多上游实现负载均衡
# [[upstream]]
# enable = true
# name = "backup-upstream"
# base_url = "https://another-provider.com/api/anthropic"
# model = "claude-3-5-sonnet-20241022"
# api_keys = ["your_key"]
# user_agent_claude = "Claude-Code/1.0.84 (Linux; Android 14)"
# user_agent_codex = "Codex/0.31.0 (Linux; Android 14)" # 对 openai_responses / openai_chat 均生效
# mode = ["anthropic", "openai_responses"]  # 可选: "anthropic" | "openai_responses" | "openai_chat"

[optimizations]
enable_network_probe_mock = true
enable_fast_prefix_detection = true
enable_historical_analysis_mock = true
enable_title_generation_skip = true
enable_suggestion_mode_skip = true
enable_filepath_extraction_mock = true

```

### ▶️ 运行

```bash
# 使用默认配置（config.toml）
cargo run -p cli_req_refiner

# 指定配置文件
cargo run -p cli_req_refiner -- /path/to/config.toml
```

服务默认监听 `0.0.0.0:9077`，可通过 `config.toml` 中的 `[server].port` 修改。
`server.force_upstream_index` 默认为空数组 `[]`，表示按原有双层轮询负载均衡；当设置为 `[0, 1]` 等非空数组时，会忽略 `enable` 字段，仅在指定的 `[[upstream]]` 下标列表内进行轮询。
旧版顶层 `port`、`log_req_body`、`log_res_body`、`user_agent_global_*` 仍兼容读取，推荐逐步迁移到 `[server]` 配置块。

---

## 📖 配置参考

### 🔌 上游配置

| 字段 | 类型 | 说明 |
|:-----|:-----|:-----|
| `name` | `String` | 上游名称，可选，主要用于日志与排障 |
| `base_url` | `String` | 上游 API 地址 |
| `model` | `String` | 强制使用的模型名称 |
| `api_keys` | `Vec<String>` | API Key 列表 — 支持多 Key 负载均衡 |
| `user_agent_claude` | `String` | 可选，该 upstream 在 Claude 接口（`anthropic` 模式）下使用的 `User-Agent`；优先级高于全局 `user_agent_global_claude` |
| `user_agent_codex` | `String` | 可选，该 upstream 在 OpenAI 接口（`openai_responses` 与 `openai_chat` 模式）下使用的 `User-Agent`；优先级高于全局 `user_agent_global_codex` |

### 🌍 全局请求头配置

| 字段 | 类型 | 说明 |
|:-----|:-----|:-----|
| `server.user_agent_global_claude` | `String` | 可选，仅对 Claude 接口（`anthropic` 模式）生效的全局 `User-Agent` |
| `server.user_agent_global_codex` | `String` | 可选，仅对 OpenAI 接口（`openai_responses` 与 `openai_chat` 模式）生效的全局 `User-Agent` |
| `server.bypass_ip_rate_limit` | `bool` | 默认 `false`，开启后每次上游尝试（含重试）覆盖 `X-Forwarded-For`，使用不同的文档 IPv6 地址；适用于所有转发协议 |

`bypass_ip_rate_limit` 支持配置热重载，对后续请求生效。开启后使用 `2001:db8::/32` 中的地址作为 IP 计数标识，不会连接这些地址；关闭时保留原始请求头行为。仅用于自己管理且信任代理请求头的上游：上游若忽略或覆盖此头，该功能无效，也不能绕过用户、令牌、模型或上游账户的限流。它只作用于经过本代理的请求，不影响浏览器直接访问 new-api 的登录请求。

### 🌐 服务监听配置

| 字段 | 类型 | 默认值 | 说明 |
|:-----|:-----|:-------|:-----|
| `server.port` | `u16` | `9077` | 服务监听端口，修改后需重启进程生效 |
| `server.force_upstream_index` | `Vec<usize>` | `[]` | 强制轮询的 upstream 下标列表；非空时忽略 `enable` 字段，仅在列表内轮询；空数组按默认双层轮询负载均衡 |
| `server.log_req_body` | `bool` | `false` | 是否打印请求体 |
| `server.log_res_body` | `bool` | `false` | 是否打印响应体 |

### ⚙️ 优化配置

| 字段 | 类型 | 默认值 | 说明 |
|:-----|:-----|:-------|:-----|
| `enable_network_probe_mock` | `bool` | `true` | 拦截配额探测请求 |
| `enable_fast_prefix_detection` | `bool` | `true` | 快速前缀检测优化 |
| `enable_historical_analysis_mock` | `bool` | `true` | 跳过历史分析请求 |
| `enable_title_generation_skip` | `bool` | `true` | 跳过标题生成请求 |
| `enable_suggestion_mode_skip` | `bool` | `true` | 跳过建议模式请求 |
| `enable_filepath_extraction_mock` | `bool` | `true` | 文件路径提取优化 |

---

## 🏗️ 工作原理

```
┌─────────────┐     ┌──────────────┐     ┌─────────────────┐
│   客户端    │────▶│CliReqRefiner │────▶│  上游 1         │
│             │     │              │     │  (API Key 1)    │
│             │     │  ✂️ 请求体   │     │  (API Key 2)    │
│             │     │    精炼      │     ├─────────────────┤
│             │     │  🔄 负载    │────▶│  上游 2         │
│             │     │    均衡     │     │  (API Key 1)    │
│             │     │  ⚡ 本地    │     │  ...            │
│             │     │    优化     │     │                 │
│             │     │  📊 Token   │     │                 │
│             │     │    统计     │     │                 │
└─────────────┘     └──────────────┘     └─────────────────┘
```

---

## 🛠️ 技术栈

| 技术 | 说明 |
|:-----|:-----|
| **[Salvo](https://salvo.rs/)** | 高性能异步 Web 框架 |
| **[Hyper](https://hyper.rs/)** | 成熟的 HTTP/1.1 和 HTTP/2 实现 |
| **[hyper-rustls](https://docs.rs/hyper-rustls/)** | TLS 支持（webpki-roots） |
| **[Tokio](https://tokio.rs/)** | Rust 异步运行时核心 |
| **[arc-swap](https://docs.rs/arc-swap/)** | 无锁热重载配置 |
| **[notify](https://docs.rs/notify/)** | 跨平台文件监听 |
| **[mimalloc](https://github.com/microsoft/mimalloc)** | 高性能内存分配器 |

---

## ⚡ 性能

- ✅ Release 构建启用 LTO（链接时优化）
- ✅ mimalloc 替换默认内存分配器
- ✅ HTTP 连接复用，减少连接开销
- ✅ 无锁配置更新，无锁竞争

---

## 🏘️ 社区

本项目最初在 [LINUX DO](https://linux.do/) 分享 — 如果你是社区的朋友，欢迎来帖子下交流反馈，也期待有感兴趣的开发者一起完善这个项目 🤝

---

## 📄 许可证

GNU General Public License v3.0 (GPLv3)
