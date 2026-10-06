import { useEffect, useRef, useState, type MouseEvent as ReactMouseEvent } from "react";
import ContextMenu, { type MenuItem, type MenuState } from "./components/ContextMenu";
import InputArea from "./components/InputArea";
import MessageList from "./components/MessageList";
import Sidebar from "./components/Sidebar";
import TitleBar from "./components/TitleBar";
import { useTheme, type Theme } from "./hooks/useTheme";
import SettingsPage from "./pages/SettingsPage";
import { StoreProvider, useStore } from "./store";

function basename(p: string): string {
  const parts = p.split(/[\\/]/).filter(Boolean);
  return parts[parts.length - 1] ?? p;
}

function Shell({ theme, setTheme }: { theme: Theme; setTheme: (t: Theme) => void }) {
  const {
    busy,
    sendMessage,
    stop,
    activeSession,
    models,
    selectedModel,
    setModel,
    workspace,
    startRename,
    deleteSession,
    removeWorkspace,
  } = useStore();
  const [view, setView] = useState<"chat" | "settings">("chat");
  const [menu, setMenu] = useState<MenuState | null>(null);
  const [confirmWs, setConfirmWs] = useState<string | null>(null);
  const scrollRef = useRef<HTMLDivElement>(null);

  useEffect(() => {
    if (view !== "chat") return;
    const el = scrollRef.current;
    if (!el) return;
    const nearBottom = el.scrollHeight - el.scrollTop - el.clientHeight < 140;
    if (nearBottom) {
      el.scrollTop = el.scrollHeight;
    }
  }, [activeSession?.messages, view]);

  function onContextMenu(e: ReactMouseEvent) {
    const target = e.target as HTMLElement;
    if (target.closest("input, textarea, [contenteditable]")) return;
    const selection = window.getSelection();
    if (selection && !selection.isCollapsed) return;

    const sessionEl = target.closest<HTMLElement>("[data-ctx-session]");
    if (sessionEl) {
      e.preventDefault();
      const id = sessionEl.dataset.ctxSession ?? "";
      setMenu({
        x: e.clientX,
        y: e.clientY,
        items: [
          { label: "重命名", onClick: () => startRename(id) },
          { label: "删除", danger: true, onClick: () => deleteSession(id) },
        ],
      });
      return;
    }

    const wsEl = target.closest<HTMLElement>("[data-ctx-workspace]");
    if (wsEl) {
      const path = wsEl.dataset.ctxWorkspace ?? "";
      const items: MenuItem[] = [];
      if (path !== workspace) {
        items.push({
          label: "删除",
          danger: true,
          onClick: () => setConfirmWs(path),
        });
      }
      if (items.length > 0) {
        e.preventDefault();
        setMenu({ x: e.clientX, y: e.clientY, items });
      }
      return;
    }

    e.preventDefault();
  }

  return (
    <div
      className="flex h-screen flex-col overflow-hidden bg-surface text-foreground"
      onContextMenu={onContextMenu}
    >
      <TitleBar />

      <div className="flex min-h-0 flex-1">
        <Sidebar
          settingsActive={view === "settings"}
          onOpenSettings={() => setView("settings")}
        />

        <main className="flex min-w-0 flex-1 flex-col">
          {view === "settings" ? (
            <div className="min-h-0 flex-1 overflow-y-auto">
              <SettingsPage
                onClose={() => setView("chat")}
                theme={theme}
                setTheme={setTheme}
              />
            </div>
          ) : (
            <>
              <div ref={scrollRef} className="min-h-0 flex-1 overflow-y-auto">
                <MessageList />
              </div>
              <InputArea
                busy={busy}
                onSend={sendMessage}
                onStop={stop}
                models={models}
                selectedModel={selectedModel}
                onModelChange={setModel}
              />
            </>
          )}
        </main>
      </div>

      <ContextMenu state={menu} onClose={() => setMenu(null)} />

      {confirmWs && (
        <div
          className="fixed inset-0 z-50 flex items-center justify-center bg-black/30"
          onClick={() => setConfirmWs(null)}
        >
          <div
            role="alertdialog"
            aria-modal="true"
            aria-label="删除工作区"
            className="w-80 rounded-card border border-line bg-surface p-5 shadow-xl"
            onClick={(e) => e.stopPropagation()}
          >
            <h3 className="text-sm font-semibold">删除工作区</h3>
            <p className="mt-2 text-sm leading-relaxed text-subtle">
              将移除「{basename(confirmWs)}」的全部对话记录，磁盘文件不受影响。
            </p>
            <div className="mt-4 flex justify-end gap-2">
              <button
                type="button"
                onClick={() => setConfirmWs(null)}
                className="rounded-full border border-line px-4 py-1.5 text-sm text-subtle transition-colors hover:text-foreground"
              >
                取消
              </button>
              <button
                type="button"
                onClick={() => {
                  removeWorkspace(confirmWs);
                  setConfirmWs(null);
                }}
                className="rounded-full bg-danger px-4 py-1.5 text-sm text-white transition-colors hover:opacity-90"
              >
                删除
              </button>
            </div>
          </div>
        </div>
      )}
    </div>
  );
}

export default function App() {
  const { theme, setTheme } = useTheme();

  return (
    <StoreProvider>
      <Shell theme={theme} setTheme={setTheme} />
    </StoreProvider>
  );
}
