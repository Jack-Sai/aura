# Aura

本地优先的桌面 AI Agent 应用，基于 Tauri 2.0 + React 19 + TypeScript 构建。通过 OpenRouter 调用 Nemotron 系列模型，支持多工作区会话、工具动作流展示与本地持久化。

## 功能

- 多工作区对话：侧边栏按工作区组织会话，支持新建、切换与删除
- 流式输出：SSE 增量渲染，打字机效果，随时停止生成
- 动作流：展示命令、读写文件、子任务等 Agent 动作，含思考过程
- 模型选择：输入区随时切换 Nemotron 模型（默认全局记忆）
- 设置页：亮/暗主题、全局规则、OpenRouter API Key 配置
- 本地持久化：会话历史、设置与密钥存于本地 SQLite

## 开发

```bash
pnpm install
pnpm tauri dev
```

前端单独调试：

```bash
pnpm dev
```

## 构建

```bash
pnpm tauri build
```

## 配置

API Key 二选一：

- 设置页「模型服务」中填写（保存在本地数据库，优先级高）
- 环境变量 `OPENROUTER_API_KEY`

## 技术栈

- [Tauri 2.0](https://tauri.app) — Rust 后端、窗口与 IPC
- React 19 + TypeScript + Vite — 前端
- Tailwind CSS v4 — 样式（Pinguo Design System 令牌）
- rusqlite — 本地存储
- OpenRouter — 模型接入
