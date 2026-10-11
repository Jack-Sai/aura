import { useState } from "react";
import { PanelRightClose, PanelRightOpen } from "lucide-react";
import { useStore } from "../store";
import { showsTodoPanel } from "../lib/mode";
import TodoPanel from "./TodoPanel";

const COLLAPSE_KEY = "aura-right-panel-collapsed";

/**
 * 右侧任务面板栏。
 *
 * 此前清单放在左侧侧边栏，与会话列表挤在一起，且多个会话各���有清单时
 * 难以区分归属。移到右侧独立栏后：
 *
 * - 位置固定，不随对话流滚动，长任务不用向上翻找
 * - 只显示**当前会话**的清单，归属明确
 * - 与会话列表分离，两者不再互相挤占空间
 *
 * 无清单时整体不渲染，避免长期占用横向空间。
 */
export default function RightPanel() {
  const { sessions, activeId, mode, deleteTodoStep, addTodoStep, setTodoStatus } =
    useStore();
  const [collapsed, setCollapsed] = useState(
    () => localStorage.getItem(COLLAPSE_KEY) === "1",
  );

  const activeSession = sessions.find((s) => s.id === activeId);
  const todos = activeSession?.todos ?? [];
  const visible = showsTodoPanel(mode, todos.length);

  if (!visible) return null;

  function toggle() {
    setCollapsed((v) => {
      localStorage.setItem(COLLAPSE_KEY, v ? "0" : "1");
      return !v;
    });
  }

  if (collapsed) {
    return (
      <aside className="flex w-11 shrink-0 flex-col items-center border-l border-line">
        <button
          type="button"
          onClick={toggle}
          aria-label="展开任务面板"
          title="展开任务面板"
          className="mt-3 rounded-tag p-1.5 text-subtle transition-colors hover:bg-bubble hover:text-foreground"
        >
          <PanelRightOpen size={14} />
        </button>
        <span
          className="mt-2 text-[10px] text-subtle tabular-nums"
          style={{ writingMode: "vertical-rl" }}
        >
          任务 {todos.length}
        </span>
      </aside>
    );
  }

  return (
    <aside className="flex w-64 shrink-0 flex-col overflow-y-auto border-l border-line">
      <button
        type="button"
        onClick={toggle}
        aria-label="收起任务面板"
        title="收起任务面板"
        className="mx-3 mt-3 mb-1 flex items-center gap-1.5 rounded-tag px-1.5 py-1 text-left text-[11px] font-medium tracking-[0.08em] text-subtle transition-colors hover:bg-bubble"
      >
        <PanelRightClose size={12} aria-hidden />
        任务面板
      </button>
      <TodoPanel
        todos={todos}
        onDeleteStep={deleteTodoStep}
        onToggleStep={setTodoStatus}
        onAddStep={addTodoStep}
      />
    </aside>
  );
}