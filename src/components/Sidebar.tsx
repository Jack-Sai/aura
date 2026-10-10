import { open } from "@tauri-apps/plugin-dialog";
import {
  ChevronRight,
  PanelLeftClose,
  PanelLeftOpen,
  Plus,
  Settings,
  Trash2,
} from "lucide-react";
import { useEffect, useState } from "react";
import { useStore } from "../store";

const COLLAPSE_KEY = "aura-sidebar-collapsed";

interface Props {
  onOpenSettings: () => void;
  /** 选中会话/工作区后回调，用于从设置页切回对话视图 */
  onNavigate?: () => void;
  settingsActive?: boolean;
}

function basename(p: string): string {
  const parts = p.split(/[\\/]/).filter(Boolean);
  return parts[parts.length - 1] ?? p;
}

export default function Sidebar({ onOpenSettings, onNavigate, settingsActive }: Props) {
  const {
    workspaces,
    workspace,
    sessions,
    activeId,
    switchWorkspace,
    newSession,
    openSession,
    deleteSession,
    renamingId,
    cancelRename,
    renameSession,
  } = useStore();

  const [expanded, setExpanded] = useState<Set<string>>(() => new Set());
  const [collapsed, setCollapsed] = useState(
    () => localStorage.getItem(COLLAPSE_KEY) === "1",
  );

  function toggleCollapsed() {
    setCollapsed((v) => {
      localStorage.setItem(COLLAPSE_KEY, v ? "0" : "1");
      return !v;
    });
  }

  useEffect(() => {
    if (workspace) {
      setExpanded((prev) => new Set(prev).add(workspace));
    }
  }, [workspace]);

  function toggleExpand(w: string) {
    setExpanded((prev) => {
      const next = new Set(prev);
      if (next.has(w)) next.delete(w);
      else next.add(w);
      return next;
    });
  }

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

  if (collapsed) {
    return (
      <aside className="flex w-14 shrink-0 flex-col border-r border-line">
        <button
          type="button"
          onClick={toggleCollapsed}
          aria-label="展开侧边栏"
          title="展开侧边栏"
          className="mx-auto mt-3 mb-1 rounded-tag p-1.5 text-subtle transition-colors hover:bg-bubble hover:text-foreground"
        >
          <img src="/logo.png" alt="Aura" draggable={false} className="h-5 w-5" />
        </button>

        <button
          type="button"
          onClick={toggleCollapsed}
          aria-label="展开侧边栏"
          title="展开侧边栏"
          className="mx-auto mb-2 rounded-tag p-1 text-subtle transition-colors hover:bg-bubble hover:text-foreground"
        >
          <PanelLeftOpen size={14} />
        </button>

        <div className="scrollbar-hidden min-h-0 flex-1 overflow-y-auto px-2 py-1">
          {workspaces.map((w) => (
            <button
              key={w}
              type="button"
              onClick={() => {
                switchWorkspace(w);
                onNavigate?.();
              }}
              title={w}
              aria-label={w}
              className="mx-auto flex w-full items-center justify-center rounded-tag py-2 transition-colors hover:bg-bubble"
            >
              <span
                className={`h-2 w-2 rounded-full ${
                  w === workspace ? "bg-brand" : "bg-line"
                }`}
              />
            </button>
          ))}
          <button
            type="button"
            onClick={pickWorkspace}
            aria-label="添加工作区"
            title="添加工作区"
            className="mx-auto flex w-full items-center justify-center rounded-tag py-2 text-subtle transition-colors hover:bg-bubble hover:text-foreground"
          >
            <Plus size={14} />
          </button>
        </div>

        <div className="px-2 py-2">
          <button
            type="button"
            onClick={onOpenSettings}
            aria-label="设置"
            title="设置"
            className={`mx-auto flex w-full items-center justify-center rounded-tag py-1.5 transition-colors hover:bg-bubble hover:text-foreground ${
              settingsActive ? "bg-bubble text-foreground" : "text-subtle"
            }`}
          >
            <Settings size={14} />
          </button>
        </div>
      </aside>
    );
  }

  return (
    <aside className="flex w-64 shrink-0 flex-col border-r border-line">
      <header className="flex items-center gap-2 px-4 py-3">
        <img
          src="/logo.png"
          alt=""
          draggable={false}
          className="h-5 w-5 shrink-0"
        />
        <span className="text-base font-semibold tracking-[-0.02em]">
          Aura
        </span>
        <button
          type="button"
          onClick={toggleCollapsed}
          aria-label="收起侧边栏"
          title="收起侧边栏"
          className="ml-auto rounded-tag p-1 text-subtle transition-colors hover:bg-bubble hover:text-foreground"
        >
          <PanelLeftClose size={14} />
        </button>
      </header>

      <div className="flex items-center justify-between px-4 pb-1">
        <span className="text-[11px] font-medium tracking-[0.08em] text-subtle">
          工作区
        </span>
        <button
          type="button"
          onClick={pickWorkspace}
          aria-label="添加工作区"
          className="rounded-tag p-0.5 text-subtle transition-colors hover:bg-bubble hover:text-foreground"
        >
          <Plus size={14} />
        </button>
      </div>

      <div className="scrollbar-hidden min-h-0 flex-1 overflow-y-auto px-3 pb-4">
        {workspaces.map((w) => {
          const isOpen = expanded.has(w);
          const wsSessions = sessions.filter((s) => s.workspace === w);
          const isCurrent = w === workspace;
          return (
            <section key={w} className="mb-0.5">
              <div
                data-ctx-workspace={w}
                className={`group flex items-center rounded-tag transition-colors ${
                  isCurrent ? "bg-bubble" : "hover:bg-bubble"
                }`}
              >
                <button
                  type="button"
                  onClick={() => toggleExpand(w)}
                  aria-label={isOpen ? "折叠工作区" : "展开工作区"}
                  className="flex w-6 shrink-0 items-center justify-center py-1.5 text-subtle"
                >
                  <ChevronRight
                    size={13}
                    className={`transition-transform ${isOpen ? "rotate-90" : ""}`}
                  />
                </button>
                <button
                  type="button"
                  onClick={() => {
                    switchWorkspace(w);
                    onNavigate?.();
                  }}
                  title={w}
                  className="flex min-w-0 flex-1 items-center gap-2 py-1.5 text-left text-sm"
                >
                  <span
                    className={`h-1.5 w-1.5 shrink-0 rounded-full ${
                      isCurrent ? "bg-brand" : "bg-line"
                    }`}
                  />
                  <span
                    className={`truncate ${
                      isCurrent ? "font-medium" : "text-subtle"
                    }`}
                  >
                    {basename(w)}
                  </span>
                </button>
                <button
                  type="button"
                  onClick={() => {
                    newSession(w);
                    onNavigate?.();
                  }}
                  aria-label="在此工作区新建对话"
                  className="mr-1 rounded-tag p-1 text-subtle opacity-0 transition-all hover:bg-surface hover:text-foreground group-hover:opacity-100"
                >
                  <Plus size={13} />
                </button>
              </div>

              {isOpen && (
                <ul className="ml-5 mt-0.5 space-y-0.5">
                  {wsSessions.map((s) => (
                    <li key={s.id}>
                      <div
                        data-ctx-session={s.id}
                        className={`group flex items-center rounded-tag transition-colors ${
                          s.id === activeId
                            ? "bg-bubble"
                            : "hover:bg-bubble"
                        }`}
                      >
                        {renamingId === s.id ? (
                          <input
                            autoFocus
                            defaultValue={s.title}
                            onFocus={(e) => e.target.select()}
                            onKeyDown={(e) => {
                              if (e.key === "Enter") {
                                renameSession(s.id, e.currentTarget.value);
                                cancelRename();
                              } else if (e.key === "Escape") {
                                cancelRename();
                              }
                            }}
                            onBlur={(e) => {
                              renameSession(s.id, e.currentTarget.value);
                              cancelRename();
                            }}
                            className="min-w-0 flex-1 rounded-lg border border-brand bg-surface px-2 py-1.5 text-sm text-foreground outline-none"
                          />
                        ) : (
                          <button
                            type="button"
                            onClick={() => {
                              openSession(s.id);
                              onNavigate?.();
                            }}
                            className="min-w-0 flex-1 truncate px-2 py-1.5 text-left text-sm"
                          >
                            {s.title}
                          </button>
                        )}
                        {renamingId !== s.id && (
                          <button
                            type="button"
                            onClick={() => deleteSession(s.id)}
                            aria-label="删除对话"
                            className="mr-1 rounded-tag p-1 text-subtle opacity-0 transition-opacity hover:text-foreground group-hover:opacity-100"
                          >
                            <Trash2 size={13} />
                          </button>
                        )}
                      </div>
                    </li>
                  ))}
                  {wsSessions.length === 0 && (
                    <li className="px-2 py-1.5 text-xs text-subtle">
                      暂无对话
                    </li>
                  )}
                </ul>
              )}
            </section>
          );
        })}
      </div>

      <footer className="px-3 py-2">
        <button
          type="button"
          onClick={onOpenSettings}
          className={`flex w-full items-center gap-2 rounded-tag px-2 py-1.5 text-sm transition-colors hover:bg-bubble hover:text-foreground ${
            settingsActive
              ? "bg-bubble font-medium text-foreground"
              : "text-subtle"
          }`}
        >
          <Settings size={14} />
          设置
        </button>
      </footer>
    </aside>
  );
}
