import { getVersion } from "@tauri-apps/api/app";
import { getCurrentWindow } from "@tauri-apps/api/window";
import { Minus, Square, X } from "lucide-react";
import { useEffect, useState } from "react";

const appWindow = getCurrentWindow();

export default function TitleBar() {
  const [version, setVersion] = useState("");

  useEffect(() => {
    getVersion()
      .then((v) => setVersion(v))
      .catch(() => setVersion("0.2.1"));
  }, []);

  return (
    <header
      data-tauri-drag-region
      className="flex h-9 shrink-0 select-none items-center justify-between border-b border-line bg-surface pl-4"
    >
      <div data-tauri-drag-region className="flex items-baseline gap-2">
        <span className="text-[13px] font-semibold tracking-[-0.02em]">
          Aura
        </span>
        <span className="font-mono text-[11px] text-subtle">v{version}</span>
      </div>

      <div className="flex h-full">
        <button
          type="button"
          onClick={() => appWindow.minimize()}
          aria-label="最小化"
          className="flex h-full w-11 items-center justify-center text-subtle transition-colors hover:bg-bubble hover:text-foreground"
        >
          <Minus size={14} />
        </button>
        <button
          type="button"
          onClick={() => appWindow.toggleMaximize()}
          aria-label="最大化"
          className="flex h-full w-11 items-center justify-center text-subtle transition-colors hover:bg-bubble hover:text-foreground"
        >
          <Square size={11} />
        </button>
        <button
          type="button"
          onClick={() => appWindow.close()}
          aria-label="关闭"
          className="flex h-full w-11 items-center justify-center text-subtle transition-colors hover:bg-danger hover:text-white dark:hover:text-surface"
        >
          <X size={14} />
        </button>
      </div>
    </header>
  );
}
