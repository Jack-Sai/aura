import { useState } from "react";
import InputArea from "./components/InputArea";
import MessageList from "./components/MessageList";
import SettingsModal from "./components/SettingsModal";
import Sidebar from "./components/Sidebar";
import { useTheme, type Theme } from "./hooks/useTheme";
import { StoreProvider, useStore } from "./store";

function Shell({ theme, onToggleTheme }: { theme: Theme; onToggleTheme: () => void }) {
  const { busy, sendMessage, stop } = useStore();
  const [settingsOpen, setSettingsOpen] = useState(false);

  return (
    <div className="flex h-screen overflow-hidden bg-surface text-foreground">
      <Sidebar
        theme={theme}
        onToggleTheme={onToggleTheme}
        onOpenSettings={() => setSettingsOpen(true)}
      />

      <main className="flex min-w-0 flex-1 flex-col">
        <div className="min-h-0 flex-1 overflow-y-auto">
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
