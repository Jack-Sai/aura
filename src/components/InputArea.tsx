import { useEffect, useRef, useState, type KeyboardEvent } from "react";
import { ArrowUp, ChevronDown, Square } from "lucide-react";
import type { ModelInfo } from "../lib/api";

interface Props {
  onSend: (text: string) => void;
  onStop: () => void;
  busy: boolean;
  models: ModelInfo[];
  selectedModel: string;
  onModelChange: (id: string) => void;
}

export default function InputArea({
  onSend,
  onStop,
  busy,
  models,
  selectedModel,
  onModelChange,
}: Props) {
  const [value, setValue] = useState("");
  const textareaRef = useRef<HTMLTextAreaElement>(null);

  useEffect(() => {
    const el = textareaRef.current;
    if (!el) return;
    el.style.height = "auto";
    const max = Math.floor(window.innerHeight / 3);
    el.style.height = `${Math.min(el.scrollHeight, max)}px`;
  }, [value]);

  function submit() {
    const text = value.trim();
    if (!text || busy) return;
    onSend(text);
    setValue("");
  }

  function handleKeyDown(e: KeyboardEvent<HTMLTextAreaElement>) {
    if (e.nativeEvent.isComposing) return;
    if (e.key === "Enter" && !e.shiftKey) {
      e.preventDefault();
      submit();
    }
  }

  return (
    <div className="px-6 pb-5 pt-3">
      <div className="mx-auto flex w-full max-w-3xl items-end gap-2 rounded-card border border-line bg-surface px-4 py-3 shadow-xs transition-colors focus-within:border-brand focus-within:shadow-sm">
        <textarea
          ref={textareaRef}
          rows={1}
          value={value}
          onChange={(e) => setValue(e.target.value)}
          onKeyDown={handleKeyDown}
          placeholder="输入消息…"
          className="max-h-[33vh] min-h-[24px] flex-1 resize-none bg-transparent text-[15px] leading-relaxed outline-none placeholder:text-subtle"
        />
        {busy ? (
          <button
            type="button"
            onClick={onStop}
            aria-label="停止生成"
            className="rounded-full border border-line p-2 text-subtle transition-colors hover:bg-bubble hover:text-foreground"
          >
            <Square size={14} />
          </button>
        ) : (
          <button
            type="button"
            onClick={submit}
            disabled={!value.trim()}
            aria-label="发送"
            className="rounded-full bg-brand p-2 text-white transition-colors enabled:hover:bg-brand-strong disabled:opacity-30"
          >
            <ArrowUp size={14} />
          </button>
        )}
      </div>
      <div className="mx-auto mt-2 flex w-full max-w-3xl items-center justify-between">
        <div className="relative">
          <select
            value={selectedModel}
            onChange={(e) => onModelChange(e.target.value)}
            disabled={models.length === 0}
            aria-label="选择模型"
            className="appearance-none rounded-full border border-line bg-surface py-1 pl-3 pr-7 text-xs text-subtle outline-none transition-colors hover:text-foreground focus:border-brand disabled:opacity-60"
          >
            {models.length === 0 && <option value="">加载中…</option>}
            {models.map((m) => (
              <option key={m.id} value={m.id}>
                {m.label}
              </option>
            ))}
          </select>
          <ChevronDown
            size={12}
            className="pointer-events-none absolute right-2.5 top-1/2 -translate-y-1/2 text-subtle"
          />
        </div>
        <p className="text-xs text-subtle">Enter 发送 · Shift+Enter 换行</p>
      </div>
    </div>
  );
}
