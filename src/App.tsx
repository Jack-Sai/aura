import { useEffect, useRef, useState } from "react";
import InputArea from "./components/InputArea";
import MessageList from "./components/MessageList";
import SettingsModal from "./components/SettingsModal";
import Sidebar from "./components/Sidebar";
import { useTheme, type Theme } from "./hooks/useTheme";
import { StoreProvider, useStore } from "./store";

function Shell({ theme, onToggleTheme }: { theme: Theme; onToggleTheme: () => void }) {
  const { busy, sendMessage, stop, activeSession } = useStore();
  const [settingsOpen, setSettingsOpen] = useState(false);
  const scrollRef = useRef<HTMLDivElement>(null);

  useEffect(() => {
    const el = scrollRef.current;
    if (!el) return;
    const nearBottom = el.scrollHeight - el.scrollTop - el.clientHeight < 140;
    if (nearBottom) {
      el.scrollTop = el.scrollHeight;
    }
  }, [activeSession?.messages]);

  return (
    <div className="flex h-screen overflow-hidden bg-surface text-foreground">
      <Sidebar
        theme={theme}
        onToggleTheme={onToggleTheme}
        onOpenSettings={() => setSettingsOpen(true)}
      />

      <main className="flex min-w-0 flex-1 flex-col">
        <div ref={scrollRef} className="min-h-0 flex-1 overflow-y-auto">
          <MessageList />
        </div>
        <InputArea busy={busy} onSend={sendMessage} onStop={stop} />
      </main>

      {settingsOpen && <SettingsModal onClose={() => setSettingsOpen(false)} />}
    </div>
  );
}

export default function App() {
  const { theme, toggle } = useTheme();

  return (
    <StoreProvider>
      <Shell theme={theme} onToggleTheme={toggle} />
    </StoreProvider>
  );
}
