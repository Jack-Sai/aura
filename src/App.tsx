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
          <div className="flex flex-1 items-center justify-center">
            <div className="text-center">
              <h2 className="text-2xl font-semibold tracking-[-0.03em]">
                Aura
              </h2>
              <p className="mt-2 text-sm text-subtle">
                Minimal surface. Maximum logic.
              </p>
            </div>
          </div>
        </main>
      </div>
    </StoreProvider>
  );
}
