/**
 * 会话模式（v0.4.0）
 *
 * 与 Rust 侧 `src-tauri/src/agent/mode.rs` 对齐。
 *
 * 注意这里**刻意不含**「哪些工具可用」的判定——那是 Rust 侧的硬闸门，
 * 前端若复制一份就会产生两个真相：UI 显示 Plan 只读，但 Rust 放行了写入，
 * 或者反过来出现「UI 灰掉了但其实能调」的错觉。前端只负责展示与选择。
 */
export type Mode = "chat" | "plan" | "build";

export const MODES: Mode[] = ["chat", "plan", "build"];

/** 模式的中文名，用于输入框胶囊 Tab。 */
export const MODE_LABEL: Record<Mode, string> = {
  chat: "对话",
  plan: "规划",
  build: "执行",
};

/** 模式的一句话说明，作为悬浮提示。 */
export const MODE_HINT: Record<Mode, string> = {
  chat: "自由对话，可读写文件并执行命令",
  plan: "仅只读：先调研并产出计划，不改动任何文件",
  build: "按计划执行，允许写入与命令",
};

/**
 * 归一化任意输入为合法模式。
 *
 * 存盘数据可能来自旧版本或被手工改坏，任何无法识别的值都回落到 chat，
 * 绝不让整个输入区因一个坏字符串而不可用。
 */
export function normalizeMode(raw: unknown): Mode {
  return typeof raw === "string" && (MODES as string[]).includes(raw)
    ? (raw as Mode)
    : "chat";
}

/**
 * Plan / Build 模式下应当呈现任务清单面板。
 * Chat 是自由对话，不展示面板以免干扰。
 */
export function showsTodoPanel(mode: Mode, todoCount: number): boolean {
  return mode !== "chat" && todoCount > 0;
}