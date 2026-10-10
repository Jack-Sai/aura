import { Check, ChevronUp } from "lucide-react";
import { useEffect, useMemo, useRef, useState } from "react";
import type { ModelInfo } from "../lib/api";

interface Props {
  models: ModelInfo[];
  selectedModel: string;
  onChange: (id: string) => void;
}

interface ModelGroup {
  provider: string;
  models: ModelInfo[];
}

export default function ModelSelect({ models, selectedModel, onChange }: Props) {
  const [open, setOpen] = useState(false);
  const rootRef = useRef<HTMLDivElement>(null);

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

  const groups = useMemo<ModelGroup[]>(() => {
    // 只展示已收藏模型；按厂商真实分组（保持首次出现顺序）。
    // 注意不能用「与上一组比较」的相邻聚合：同一厂商的模型在数组中一旦
    // 不连续（例如先收藏 A 厂商、再收藏 B 厂商、再收藏 A 厂商），
    // 就会被拆成多个同名标题。
    const map = new Map<string, ModelInfo[]>();
    for (const m of models.filter((x) => x.pinned)) {
      const name = m.provider_name || m.provider;
      const bucket = map.get(name);
      if (bucket) bucket.push(m);
      else map.set(name, [m]);
    }
    return [...map].map(([provider, list]) => ({ provider, models: list }));
  }, [models]);

  const selected = models.find((m) => m.key === selectedModel);

  /** 模型显示名：优先后端维护的 label，回退到完整 id */
  function displayName(m: ModelInfo): string {
    return m.label.trim() || m.id;
  }

  return (
    <div ref={rootRef} className="relative">
      <button
        type="button"
        onClick={() => setOpen((v) => !v)}
        aria-label="选择模型"
        aria-expanded={open}
        disabled={models.length === 0}
        className="flex items-center gap-1.5 py-1 text-xs text-subtle transition-colors hover:text-foreground disabled:opacity-60"
      >
        <span className="break-words text-left">
          {selected ? displayName(selected) : "加载中…"}
        </span>
        <ChevronUp
          size={12}
          className={`shrink-0 transition-transform ${open ? "" : "rotate-180"}`}
        />
      </button>

      {open && (
        <div
          role="listbox"
          aria-label="模型列表"
          className="absolute bottom-full left-0 z-20 mb-1.5 max-h-[420px] min-w-[420px] overflow-y-auto overscroll-contain rounded-lg border border-line bg-surface py-1 shadow-lg"
        >
          {groups.map((g) => (
            <div key={g.provider}>
              <div className="px-3 pb-1 pt-2 text-[11px] text-subtle">
                {g.provider}
              </div>
              {g.models.map((m) => {
                const isActive = m.key === selectedModel;
                return (
                  <button
                    key={m.key}
                    type="button"
                    role="option"
                    aria-selected={isActive}
                    onClick={() => {
                      onChange(m.key);
                      setOpen(false);
                    }}
                    className={`flex w-full items-center gap-3 px-3 py-2 text-left text-sm transition-colors ${
                      isActive ? "bg-bubble" : "hover:bg-bubble"
                    }`}
                  >
                    <span className="min-w-0 flex-1 break-words">
                      {displayName(m)}
                    </span>
                    <span className="flex w-5 shrink-0 justify-end">
                      {isActive && (
                        <Check size={13} className="text-brand" />
                      )}
                    </span>
                  </button>
                );
              })}
            </div>
          ))}
        </div>
      )}
    </div>
  );
}
