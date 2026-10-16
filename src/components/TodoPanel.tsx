import { useMemo, useState } from "react";
import { Check, ChevronDown, ChevronRight, Loader2, Plus, Trash2 } from "lucide-react";
import { displayOrder, doneCount, type Todo, type TodoStatus } from "../lib/todo";

/** 折叠状态存 localStorage：跨会话共享一个偏好，符合侧边栏自身的既有做法。 */
const COLLAPSE_KEY = "aura-todo-collapsed";

interface Props {
  todos: Todo[];
  onDeleteStep?: (id: string) => void;
  /** 手动勾选/回退。模型不更新状态时的兜底 */
  onToggleStep?: (id: string, next: TodoStatus) => void;
  /** 手动新增步骤 */
  onAddStep?: (title: string) => void;
}

function StatusIcon({ status }: { status: TodoStatus }) {
  if (status === "done") {
    return <Check size={12} className="text-brand" aria-hidden />;
  }
  if (status === "in_progress") {
    return <Loader2 size={12} className="animate-spin text-brand" aria-hidden />;
  }
  return <span className="block size-3 rounded-full border border-line" aria-hidden />;
}

function statusLabel(status: TodoStatus): string {
  if (status === "done") return "已完成";
  if (status === "in_progress") return "进行中";
  return "待执行";
}

/**
 * 点击后的下一个状态：未开始→进行中，进行中→已完成，已完成→未开始。
 *
 * 做成三态循环而非「勾选框」：面板宽度有限，循环切换不占额外一行，
 * 且「进行中」这个中间态在执行模式下确实有独立含义。
 */
function nextStatus(status: TodoStatus): TodoStatus {
  if (status === "pending") return "in_progress";
  if (status === "in_progress") return "done";
  return "pending";
}

/**
 * 侧边栏任务清单面板。
 *
 * 放在侧边栏而非对话流有两个实际原因：
 * 一是长任务不用向上翻找；二是 `<todo_update>` 事件会在流式输出中途
 * 插入对话流，store 的 mergeText 只在「最后一个 block 是 text」时合并，
 * 插在中间会切断 Markdown 段落。放侧边栏从根上规避了这个既有问题。
 */
export default function TodoPanel({
  todos,
  onDeleteStep,
  onToggleStep,
  onAddStep,
}: Props) {
  // 自管理折叠态：做成受控 prop 却没有任何父组件管理它，
  // 等于把状态管理责任推给每个调用方，得不到好处
  const [collapsed, setCollapsed] = useState(
    () => localStorage.getItem(COLLAPSE_KEY) === "1",
  );
  const [draft, setDraft] = useState("");

  function toggleCollapsed() {
    setCollapsed((v) => {
      localStorage.setItem(COLLAPSE_KEY, v ? "0" : "1");
      return !v;
    });
  }

  function submitDraft() {
    const text = draft.trim();
    if (!text || !onAddStep) return;
    onAddStep(text);
    setDraft("");
  }

  const ordered = useMemo(() => displayOrder(todos), [todos]);
  const done = doneCount(todos);

  if (todos.length === 0) return null;

  return (
    <section
      aria-label="任务清单"
      className="mx-3 mb-3 rounded-card border border-line bg-surface"
    >
      <button
        type="button"
        onClick={toggleCollapsed}
        aria-expanded={!collapsed}
        className="flex w-full items-center gap-2 px-3 py-2 text-left"
      >
        {collapsed ? (
          <ChevronRight size={13} className="text-subtle" aria-hidden />
        ) : (
          <ChevronDown size={13} className="text-subtle" aria-hidden />
        )}
        <span className="text-xs font-medium">任务清单</span>
        <span className="ml-auto text-[11px] text-subtle tabular-nums">
          {done}/{todos.length}
        </span>
      </button>

      {!collapsed && (
        <div className="px-3 pb-3">
          <ol className="flex flex-col gap-1.5">
            {ordered.map((t) => (
              <li
                key={t.id}
                className={[
                  "group flex items-start gap-2 rounded-md px-1.5 py-1 text-[12px] leading-snug transition-colors",
                  t.status === "in_progress"
                    ? "bg-bubble text-foreground"
                    : "text-subtle",
                ].join(" ")}
              >
                <button
                  type="button"
                  aria-label={`步骤状态：${statusLabel(t.status)}`}
                  onClick={onToggleStep ? () => onToggleStep(t.id, nextStatus(t.status)) : undefined}
                  disabled={!onToggleStep}
                  className="mt-0.5 shrink-0 transition-transform disabled:cursor-default enabled:hover:scale-110"
                >
                  <StatusIcon status={t.status} />
                </button>
                <span className="min-w-0 flex-1">
                  <span
                    className={
                      t.status === "done"
                        ? "break-words line-through decoration-line"
                        : "break-words"
                    }
                  >
                    {t.title}
                  </span>
                  <span className="sr-only">（{statusLabel(t.status)}）</span>
                </span>
                {onDeleteStep && (
                  <button
                    type="button"
                    aria-label={`删除步骤：${t.title}`}
                    onClick={() => onDeleteStep(t.id)}
                    className="shrink-0 rounded p-0.5 text-subtle opacity-0 transition-opacity hover:text-danger focus-visible:opacity-100 group-hover:opacity-100"
                  >
                    <Trash2 size={11} aria-hidden />
                  </button>
                )}
              </li>
            ))}
          </ol>

          {onAddStep && (
            <div className="mt-2 flex items-center gap-1">
              <input
                value={draft}
                onChange={(e) => setDraft(e.target.value)}
                onKeyDown={(e) => {
                  if (e.key === "Enter") {
                    e.preventDefault();
                    submitDraft();
                  }
                }}
                placeholder="添加步骤"
                aria-label="添加步骤"
                className="min-w-0 flex-1 rounded-md border border-line bg-transparent px-2 py-1 text-xs outline-none transition-colors placeholder:text-subtle focus:border-brand"
              />
              <button
                type="button"
                aria-label="新增步骤"
                onClick={submitDraft}
                disabled={!draft.trim()}
                className="shrink-0 rounded-md border border-line p-1 text-subtle transition-colors hover:border-brand hover:text-brand disabled:cursor-not-allowed disabled:opacity-40"
              >
                <Plus size={11} aria-hidden />
              </button>
            </div>
          )}
        </div>
      )}
    </section>
  );
}