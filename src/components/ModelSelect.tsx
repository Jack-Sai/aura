import { Check, ChevronUp } from "lucide-react";
import { useEffect, useMemo, useRef, useState } from "react";
import type { ModelInfo } from "../lib/api";

interface Props {
  models: ModelInfo[];
  selectedModel: string;
  onChange: (id: string) => void;
}

export interface ParsedModel {
  vendor: string;
  name: string;
  params: string;
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
  const m = core.match(/^(.*?)-(?=\d+(?:\.\d+)?b(?:-|$))/);
  const hasSplit = Boolean(m && m[1]);
  const name = hasSplit ? (m as RegExpMatchArray)[1] : core;
  const params = hasSplit ? core.slice((m as RegExpMatchArray)[1].length + 1) : "";
  return { vendor, name, params, tags };
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
    const out: ModelGroup[] = [];
    for (const m of models) {
      const last = out[out.length - 1];
      if (last && last.provider === m.provider) last.models.push(m);
      else out.push({ provider: m.provider, models: [m] });
    }
    return out;
  }, [models]);

  const selected = models.find((m) => m.key === selectedModel);
  const parsed = selected ? parseModelId(selected.id) : null;

  return (
    <div ref={rootRef} className="relative">
      <button
        type="button"
        onClick={() => setOpen((v) => !v)}
        aria-label="选择模型"
        aria-expanded={open}
        disabled={models.length === 0}
        className="flex max-w-[240px] items-center gap-1.5 rounded-full border border-line bg-surface py-1 pl-3 pr-2.5 text-xs text-subtle transition-colors hover:text-foreground disabled:opacity-60"
      >
        <span className="truncate">
          {parsed
            ? parsed.params
              ? `${parsed.name} ${parsed.params}`
              : parsed.name
            : "加载中…"}
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
          className="absolute bottom-full left-0 z-20 mb-1.5 min-w-[420px] overflow-hidden rounded-lg border border-line bg-surface py-1 shadow-lg"
        >
          {groups.map((g) => (
            <div key={g.provider}>
              <div className="px-3 pb-1 pt-2 text-[11px] text-subtle">
                {g.provider}
              </div>
              {g.models.map((m) => {
                const p = parseModelId(m.id);
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
                    <span className="min-w-0 flex-1 truncate">{p.name}</span>
                    <span className="flex shrink-0 items-center gap-1.5">
                      {p.params && (
                        <span className="rounded-full border border-line bg-bubble px-1.5 py-0.5 text-[10px] text-subtle">
                          {p.params}
                        </span>
                      )}
                      {p.tags.map((t) => (
                        <span
                          key={t}
                          className="rounded-full border border-line bg-bubble px-1.5 py-0.5 text-[10px] text-subtle"
                        >
                          {t}
                        </span>
                      ))}
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
