import { Check, ChevronUp } from "lucide-react";
import { useEffect, useRef, useState } from "react";
import type { ModelInfo } from "../lib/api";

interface Props {
  models: ModelInfo[];
  selectedModel: string;
  onChange: (id: string) => void;
}

export interface ParsedModel {
  vendor: string;
  name: string;
  tags: string[];
}

export function parseModelId(id: string): ParsedModel {
  const slash = id.indexOf("/");
  const vendor = slash >= 0 ? id.slice(0, slash) : id;
  const rest = slash >= 0 ? id.slice(slash + 1) : id;
  const colon = rest.indexOf(":");
  const core = colon >= 0 ? rest.slice(0, colon) : rest;
  const tags = colon >= 0
    ? rest
        .slice(colon + 1)
        .split(":")
        .filter(Boolean)
    : [];
  const name = core.replace(/-(?=\d+(?:\.\d+)?b(?:-|$))/, " ");
  return { vendor, name, tags };
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

  const selected = models.find((m) => m.id === selectedModel);
  const parsed = selected ? parseModelId(selected.id) : null;

  return (
    <div ref={rootRef} className="relative">
      <button
        type="button"
        onClick={() => setOpen((v) => !v)}
        aria-label="选择模型"
        aria-expanded={open}
        disabled={models.length === 0}
        className="flex max-w-[220px] items-center gap-1.5 rounded-full border border-line bg-surface py-1 pl-3 pr-2.5 text-xs text-subtle transition-colors hover:text-foreground disabled:opacity-60"
      >
        <span className="truncate">
          {parsed ? parsed.name : "加载中…"}
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
          className="absolute bottom-full left-0 z-20 mb-1.5 min-w-[280px] overflow-hidden rounded-lg border border-line bg-surface py-1 shadow-lg"
        >
          {models.map((m) => {
            const p = parseModelId(m.id);
            const isActive = m.id === selectedModel;
            return (
              <button
                key={m.id}
                type="button"
                role="option"
                aria-selected={isActive}
                onClick={() => {
                  onChange(m.id);
                  setOpen(false);
                }}
                className={`flex w-full items-center gap-2 px-3 py-2 text-left text-sm transition-colors ${
                  isActive ? "bg-bubble" : "hover:bg-bubble"
                }`}
              >
                <span className="shrink-0 text-[11px] text-subtle">
                  {p.vendor}
                </span>
                <span className="min-w-0 flex-1 truncate">{p.name}</span>
                {p.tags.map((t) => (
                  <span
                    key={t}
                    className="shrink-0 rounded-tag border border-line bg-bubble px-1.5 py-0.5 text-[10px] text-subtle"
                  >
                    {t}
                  </span>
                ))}
                {isActive && (
                  <Check size={13} className="shrink-0 text-brand" />
                )}
              </button>
            );
          })}
        </div>
      )}
    </div>
  );
}
