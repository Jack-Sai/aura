import { useEffect, useRef } from "react";

export interface MenuItem {
  label: string;
  danger?: boolean;
  onClick: () => void;
}

export interface MenuState {
  x: number;
  y: number;
  items: MenuItem[];
}

interface Props {
  state: MenuState | null;
  onClose: () => void;
}

const MENU_WIDTH = 150;
const ITEM_HEIGHT = 30;
const EDGE_PAD = 8;

export default function ContextMenu({ state, onClose }: Props) {
  const ref = useRef<HTMLDivElement>(null);

  useEffect(() => {
    if (!state) return;
    function onDown(e: MouseEvent) {
      if (!ref.current?.contains(e.target as Node)) onClose();
    }
    function onKey(e: KeyboardEvent) {
      if (e.key === "Escape") onClose();
    }
    document.addEventListener("mousedown", onDown);
    document.addEventListener("keydown", onKey);
    return () => {
      document.removeEventListener("mousedown", onDown);
      document.removeEventListener("keydown", onKey);
    };
  }, [state, onClose]);

  if (!state) return null;

  const menuHeight = state.items.length * ITEM_HEIGHT + EDGE_PAD * 2;
  const left = Math.max(
    EDGE_PAD,
    Math.min(state.x, window.innerWidth - MENU_WIDTH - EDGE_PAD),
  );
  const top = Math.max(
    EDGE_PAD,
    Math.min(state.y, window.innerHeight - menuHeight - EDGE_PAD),
  );

  return (
    <div
      ref={ref}
      role="menu"
      style={{ left, top }}
      className="fixed z-50 min-w-[140px] overflow-hidden rounded-lg border border-line bg-surface py-1 shadow-lg"
    >
      {state.items.map((item) => (
        <button
          key={item.label}
          type="button"
          role="menuitem"
          onClick={() => {
            item.onClick();
            onClose();
          }}
          className={`flex h-[30px] w-full items-center px-3 text-left text-sm transition-colors hover:bg-bubble ${
            item.danger ? "text-danger" : "text-foreground"
          }`}
        >
          {item.label}
        </button>
      ))}
    </div>
  );
}
