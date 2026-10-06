import MessageList from "./components/MessageList";
import Sidebar from "./components/Sidebar";
import { useTheme } from "./hooks/useTheme";
import { StoreProvider } from "./store";

export default function App() {
  const { theme, toggle } = useTheme();

  return (
    <StoreProvider>
      <div className="flex h-screen overflow-hidden bg-surface text-foreground">
        <Sidebar theme={theme} onToggleTheme={toggle} />

        <main className="flex min-w-0 flex-1 flex-col">
          <div className="min-h-0 flex-1 overflow-y-auto">
            <MessageList />
          </div>
        </main>
      </div>
    </StoreProvider>
  );
}
