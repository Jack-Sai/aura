import ThemeToggle from "./components/ThemeToggle";
import { useTheme } from "./hooks/useTheme";

export default function App() {
  const { theme, toggle } = useTheme();

  return (
    <div className="flex h-screen overflow-hidden bg-surface text-foreground">
      <aside className="flex w-64 shrink-0 flex-col border-r border-line">
        <header className="flex items-center justify-between px-4 py-3">
          <span className="text-base font-semibold tracking-[-0.02em]">
            Aura
          </span>
          <ThemeToggle theme={theme} onToggle={toggle} />
        </header>
        <nav className="flex-1 overflow-y-auto px-3 pb-4" />
      </aside>

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
  );
}
