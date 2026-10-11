import { useMemo, useState } from "react";
import { Check, ChevronDown, ChevronRight, Loader2, Play, Trash2 } from "lucide-react";
import { displayOrder, doneCount, type Todo, type TodoStatus } from "../lib/todo";

/** 折叠状态存 localStorage：跨会话共享一个偏好，符合侧边栏自身的既有做法。 */
const COLLAPSE_KEY = "aura-todo-collapsed";

interface Props {
  todos: Todo[];
  /** 开始执行：把当前清单带入 Build 模式 */
  onStartBuild?: () => void;
  onDeleteStep?: (id: string) => void;
  busy?: boolean;
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
 * 侧边栏任务清单面板。
 *
 * 放在侧边栏而非对话流有两个实际原因：
 * 一是长任务不用向上翻找；二是 `<todo_update>` 事件会在流式输出中途
 * 插入对话流，store 的 mergeText 只在「最后一个 block 是 text」时合并，
 * 插在中间会切断 Markdown 段落。放侧边栏从根上规避了这个既有问题。
 */
export default function TodoPanel({
  todos,
  onStartBuild,
  onDeleteStep,
  busy,
}: Props) {
  // 自管理折叠态：做成受控 prop 却没有任何父组件管理它，
  // 等于把状态管理责任推给每个调用方，得不到好处
  const [collapsed, setCollapsed] = useState(
    () => localStorage.getItem(COLLAPSE_KEY) === "1",
  );

  function toggleCollapsed() {
    setCollapsed((v) => {
      localStorage.setItem(COLLAPSE_KEY, v ? "0" : "1");
      return !v;
    });
  }

  const ordered = useMemo(() => displayOrder(todos), [todos]);
  const done = doneCount(todos);
  const allDone = done === todos.length;

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
                <span className="mt-0.5 shrink-0">
                  <StatusIcon status={t.status} />
                </span>
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
                    className="shrink-0 rounded p-0.5 text-subtle opacity-0 transition-opacity hover:text-error focus-visible:opacity-100 group-hover:opacity-100"
                  >
                    <Trash2 size={11} aria-hidden />
                  </button>
                )}
              </li>
            ))}
          </ol>

          {onStartBuild && (
            <button
              type="button"
              onClick={onStartBuild}
              disabled={busy || allDone}
              className="mt-3 flex w-full items-center justify-center gap-1.5 rounded-md border border-line bg-bubble py-1.5 text-xs transition-colors hover:border-brand hover:text-brand disabled:cursor-not-allowed disabled:opacity-40"
            >
              <Play size={12} aria-hidden />
              {allDone ? "已全部完成" : "开始执行"}
            </button>
          )}
        </div>
      )}
    </section>
  );
}