/**
 * 任务清单的数据契约（v0.4.0）
 *
 * 与 Rust 侧 `src-tauri/src/agent/todo.rs` 的 `Todo` / `TodoStatus` 一一对应。
 *
 * 状态串必须严格匹配后端的序列化值：后端曾因 serde 的 `rename_all =
 * "lowercase"` 把 `in_progress` 写成 `inprogress`，导致落盘后读不回来。
 * 前端因此不做任何"就近归一化"，只认这三个确定值。
 */
export type TodoStatus = "pending" | "in_progress" | "done";

export interface Todo {
  id: string;
  title: string;
  status: TodoStatus;
}

export const TODO_STATUSES: TodoStatus[] = ["pending", "in_progress", "done"];

export function isTodoStatus(v: unknown): v is TodoStatus {
  return typeof v === "string" && (TODO_STATUSES as string[]).includes(v);
}

/**
 * 归一化来自后端或存盘数据的任意输入。
 *
 * 坏数据必须在这里被拦下：状态值若被当成任意字符串透传，会在渲染时
 * 落到「未知状态」分支，用户看到的是一个既非进行中也非完成的步骤。
 */
export function normalizeTodos(raw: unknown): Todo[] {
  if (!Array.isArray(raw)) return [];
  const out: Todo[] = [];
  const seen = new Set<string>();
  for (const item of raw) {
    if (!item || typeof item !== "object") continue;
    const t = item as Record<string, unknown>;
    const id = typeof t.id === "string" ? t.id.trim() : "";
    const title = typeof t.title === "string" ? t.title.trim() : "";
    if (!id || !title || seen.has(id)) continue;
    seen.add(id);
    out.push({
      id,
      title,
      status: isTodoStatus(t.status) ? t.status : "pending",
    });
  }
  return out;
}

/** 已完成步骤数。 */
export function doneCount(todos: Todo[]): number {
  return todos.filter((t) => t.status === "done").length;
}

/**
 * 面板展示顺序：进行中在前，未开始居中，已完成沉底。
 * 同组内保持原顺序——用户手动排的顺序是有意义的。
 */
export function displayOrder(todos: Todo[]): Todo[] {
  const rank: Record<TodoStatus, number> = {
    in_progress: 0,
    pending: 1,
    done: 2,
  };
  return todos
    .map((t, i) => ({ t, i }))
    .sort((a, b) => rank[a.t.status] - rank[b.t.status] || a.i - b.i)
    .map((x) => x.t);
}

/** 应用单条状态更新，返回是否命中。 */
export function applyTodoUpdate(
  todos: Todo[],
  id: string,
  status: TodoStatus,
): boolean {
  const hit = todos.find((t) => t.id === id);
  if (!hit) return false;
  hit.status = status;
  return true;
}

/**
 * 清单是否已全部完成。
 *
 * 用于「全部完成后自动清空」——任务做完即清场，避免旧清单长期占着
 * 侧边栏，让下一次任务的计划面板从空白开始。
 */
export function isListComplete(todos: Todo[]): boolean {
  return todos.length > 0 && todos.every((t) => t.status === "done");
}

/**
 * 计算更新后的清单，若已全部完成则返回空数组。
 *
 * 把「更新」与「清空」合成一步，是为了让两者共享同一次落盘：
 * 分开写会产生两次 invoke，且中间态会让面板闪一下再消失。
 */
export function applyAutoClear(todos: Todo[]): Todo[] {
  return isListComplete(todos) ? [] : todos;
}

/** 生成新步骤的 id，避开清单中已有的编号。 */
export function nextTodoId(todos: Todo[]): string {
  let n = todos.length + 1;
  const taken = new Set(todos.map((t) => t.id));
  while (taken.has(`s${n}`)) n++;
  return `s${n}`;
}