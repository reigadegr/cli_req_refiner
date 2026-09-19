### 开发调试
```bash
sh debug.sh
```

> ⚠️ 全程禁止执行 `cargo build`、`cargo build --release` 以及其他任何 `cargo build*` 构建命令。
> 如需验证或测试，请统一执行 `sh debug.sh`（已包含格式化、clippy 与 `cargo test` 的全套检查）。

## 提交说明

提交时必须遵循约定式提交（Conventional Commits）。

如未进行特额外说明，不得做任何多余的事情，只允许依据git diff/status获取信息

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
