import { useEffect, useRef, useState } from "react";
import { Check, ChevronDown } from "lucide-react";

export interface SelectOption {
  value: string;
  label: string;
}

interface Props {
  value: string;
  options: SelectOption[];
  onChange: (value: string) => void;
  /** 下拉面板最大高度，超出后内部滚动 */
  maxHeight?: number;
  className?: string;
  "aria-label"?: string;
  disabled?: boolean;
}

/**
 * 设计系统风格的下拉选择器。
 *
 * 与 ModelSelect 保持一致的结构：胶囊触发器 + 浮层弹层 + bg-bubble 选中态 +
 * 固定宽度勾选槽（避免选中/未选中时其它列抖动）。在此基础上补齐两点：
 * 1. 面板限高滚动，选项过多时不会溢出窗口
 * 2. 按可用空间自动决定向上/向下展开
 */
export default function Select({
  value,
  options,
  onChange,
  maxHeight = 320,
  className = "",
  "aria-label": ariaLabel,
  disabled,
}: Props) {
  const [open, setOpen] = useState(false);
  const [up, setUp] = useState(false);
  const rootRef = useRef<HTMLDivElement>(null);
  const triggerRef = useRef<HTMLButtonElement>(null);

  useEffect(() => {
    if (!open) return;
    function onDown(e: MouseEvent) {
      if (!rootRef.current?.contains(e.target as Node)) setOpen(false);
    }
    function onKey(e: KeyboardEvent) {
      if (e.key === "Escape") setOpen(false);
    }
    document.addEventListener("mousedown", onDown);
    document.addEventListener("keydown", onKey);
    return () => {
      document.removeEventListener("mousedown", onDown);
      document.removeEventListener("keydown", onKey);
    };
  }, [open]);

  function toggle() {
    if (disabled) return;
    if (!open) {
      // 触发器下方空间不足时改为向上展开
      const rect = triggerRef.current?.getBoundingClientRect();
      const below = window.innerHeight - (rect?.bottom ?? 0);
      setUp(below < maxHeight + 40 && (rect?.top ?? 0) > below);
    }
    setOpen((v) => !v);
  }

  const current = options.find((o) => o.value === value);

  return (
    <div ref={rootRef} className="relative">
      <button
        ref={triggerRef}
        type="button"
        role="combobox"
        aria-haspopup="listbox"
        aria-expanded={open}
        aria-label={ariaLabel}
        disabled={disabled}
        onClick={toggle}
        className={`flex w-full items-center gap-1.5 rounded-full border border-line bg-surface py-1 pl-3 pr-2.5 text-xs text-foreground transition-colors hover:bg-bubble disabled:opacity-60 ${className}`}
      >
        <span className="min-w-0 flex-1 truncate text-left">
          {current?.label ?? value ?? "—"}
        </span>
        <ChevronDown
          size={12}
          className={`shrink-0 text-subtle transition-transform ${open ? "" : "rotate-180"}`}
        />
      </button>

      {open && (
        <div
          role="listbox"
          aria-label={ariaLabel}
          style={{ maxHeight }}
          className={`absolute left-0 z-20 w-full min-w-[160px] overflow-y-auto overscroll-contain rounded-lg border border-line bg-surface py-1 shadow-lg ${
            up ? "bottom-full mb-1.5" : "top-full mt-1.5"
          }`}
        >
          {options.map((o) => {
            const active = o.value === value;
            return (
              <button
                key={o.value}
                type="button"
                role="option"
                aria-selected={active}
                onClick={() => {
                  onChange(o.value);
                  setOpen(false);
                }}
                className={`flex w-full items-center gap-2 px-3 py-1.5 text-left text-xs transition-colors ${
                  active ? "bg-bubble" : "hover:bg-bubble"
                }`}
              >
                <span className="min-w-0 flex-1 truncate">{o.label}</span>
                <span className="flex w-4 shrink-0 justify-end">
                  {active && <Check size={12} className="text-brand" />}
                </span>
              </button>
            );
          })}
        </div>
      )}
    </div>
  );
}