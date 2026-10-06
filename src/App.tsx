import { useEffect, useRef, useState } from "react";
import InputArea from "./components/InputArea";
import MessageList from "./components/MessageList";
import Sidebar from "./components/Sidebar";
import TitleBar from "./components/TitleBar";
import { useTheme, type Theme } from "./hooks/useTheme";
import SettingsPage from "./pages/SettingsPage";
import { StoreProvider, useStore } from "./store";

function Shell({ theme, setTheme }: { theme: Theme; setTheme: (t: Theme) => void }) {
  const { busy, sendMessage, stop, activeSession, models, selectedModel, setModel } =
    useStore();
  const [view, setView] = useState<"chat" | "settings">("chat");
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

  return (
    <div className="flex h-screen flex-col overflow-hidden bg-surface text-foreground">
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
