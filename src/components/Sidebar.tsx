import { open } from "@tauri-apps/plugin-dialog";
import { Plus, Settings, Trash2 } from "lucide-react";
import type { Theme } from "../hooks/useTheme";
import { useStore } from "../store";
import ThemeToggle from "./ThemeToggle";

interface Props {
  theme: Theme;
  onToggleTheme: () => void;
  onOpenSettings: () => void;
}

function basename(p: string): string {
  const parts = p.split(/[\\/]/).filter(Boolean);
  return parts[parts.length - 1] ?? p;
}

export default function Sidebar({ theme, onToggleTheme, onOpenSettings }: Props) {
  const {
    workspaces,
    workspace,
    sessions,
    activeId,
    switchWorkspace,
    newSession,
    selectSession,
    deleteSession,
  } = useStore();

  async function pickWorkspace() {
    try {
      const dir = await open({ directory: true, title: "选择工作区目录" });
      if (typeof dir === "string") {
        await switchWorkspace(dir);
      }
    } catch {
      /* 用户取消或目录无效时静默 */
    }
  }

  const wsSessions = sessions.filter((s) => s.workspace === workspace);

  return (
    <aside className="flex w-64 shrink-0 flex-col border-r border-line">
      <header className="flex items-center justify-between px-4 py-3">
        <span className="text-base font-semibold tracking-[-0.02em]">
          Aura
        </span>
        <ThemeToggle theme={theme} onToggle={onToggleTheme} />
      </header>

      <div className="px-3">
        <div className="mb-1 flex items-center justify-between px-1">
          <span className="text-xs font-medium text-subtle">工作区</span>
          <button
            type="button"
            onClick={pickWorkspace}
            aria-label="添加工作区"
            className="rounded p-0.5 text-subtle transition-colors hover:bg-bubble hover:text-foreground"
          >
            <Plus size={14} />
          </button>
        </div>
        <ul className="space-y-0.5">
          {workspaces.map((w) => (
            <li key={w}>
              <button
                type="button"
                onClick={() => switchWorkspace(w)}
                title={w}
                className={`flex w-full items-center gap-2 rounded-tag px-2 py-1.5 text-left text-sm transition-colors hover:bg-bubble ${
                  w === workspace ? "bg-bubble font-medium" : "text-subtle"
                }`}
              >
                <span
                  className={`h-1.5 w-1.5 shrink-0 rounded-full ${
                    w === workspace ? "bg-brand" : "bg-line"
                  }`}
                />
                <span className="truncate">{basename(w)}</span>
              </button>
            </li>
          ))}
        </ul>
      </div>

      <div className="mt-5 min-h-0 flex-1 overflow-y-auto px-3 pb-4">
        <div className="mb-1 flex items-center justify-between px-1">
          <span className="text-xs font-medium text-subtle">对话</span>
          <button
            type="button"
            onClick={newSession}
            aria-label="新建对话"
            className="rounded p-0.5 text-subtle transition-colors hover:bg-bubble hover:text-foreground"
          >
            <Plus size={14} />
          </button>
        </div>
        <ul className="space-y-0.5">
          {wsSessions.map((s) => (
            <li key={s.id}>
              <div
                className={`group flex items-center rounded-tag transition-colors ${
                  s.id === activeId ? "bg-bubble" : "hover:bg-bubble"
                }`}
              >
                <button
                  type="button"
                  onClick={() => selectSession(s.id)}
                  className="min-w-0 flex-1 truncate px-2 py-1.5 text-left text-sm"
                >
                  {s.title}
                </button>
                <button
                  type="button"
                  onClick={() => deleteSession(s.id)}
                  aria-label="删除对话"
                  className="mr-1 rounded p-1 text-subtle opacity-0 transition-opacity hover:text-foreground group-hover:opacity-100"
                >
                  <Trash2 size={13} />
                </button>
              </div>
            </li>
          ))}
        </ul>
        {wsSessions.length === 0 && (
          <p className="px-1 py-2 text-xs text-subtle">暂无对话</p>
        )}
      </div>

      <footer className="border-t border-line px-3 py-2">
        <button
          type="button"
          onClick={onOpenSettings}
          className="flex w-full items-center gap-2 rounded-tag px-2 py-1.5 text-sm text-subtle transition-colors hover:bg-bubble hover:text-foreground"
        >
          <Settings size={14} />
          设置
        </button>
      </footer>
    </aside>
  );
}
