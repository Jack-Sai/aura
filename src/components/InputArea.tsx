import { useEffect, useRef, useState, type KeyboardEvent } from "react";
import { ArrowUp, Square } from "lucide-react";
import type { ModelInfo } from "../lib/api";
import type { Mode } from "../lib/mode";
import ModeTabs from "./ModeTabs";
import ModelSelect from "./ModelSelect";

interface Props {
  onSend: (text: string) => void;
  onStop: () => void;
  busy: boolean;
  models: ModelInfo[];
  selectedModel: string;
  onModelChange: (id: string) => void;
  mode: Mode;
  onModeChange: (m: Mode) => void;
}

export default function InputArea({
  onSend,
  onStop,
  busy,
  models,
  selectedModel,
  onModelChange,
  mode,
  onModeChange,
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
      {/* items-center 而非 items-end：发送按钮比单行输入框高，
          底部对齐会让输入框顶部留出空隙、文字整体偏下（用户反馈的问题）。
          多行时按钮停在垂直中间也更符合直觉。 */}
      <div className="mx-auto flex w-full max-w-4xl xl:max-w-5xl 2xl:max-w-6xl items-center gap-2 rounded-card border border-line bg-surface px-4 py-3 shadow-xs transition-colors focus-within:border-brand focus-within:shadow-sm">
        <textarea
          ref={textareaRef}
          rows={1}
          value={value}
          onChange={(e) => setValue(e.target.value)}
          onKeyDown={handleKeyDown}
          placeholder={
            mode === "plan"
              ? "描述你想做的事，先让 Aura 给出计划…"
              : mode === "build"
                ? "描述要执行的任务…"
                : "输入消息…"
          }
          className="max-h-[33vh] min-h-6 flex-1 resize-none overflow-y-auto bg-transparent py-0 text-[15px] leading-6 outline-none placeholder:text-subtle"
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
      <div className="mx-auto mt-2 flex w-full max-w-4xl xl:max-w-5xl 2xl:max-w-6xl items-center justify-between gap-3">
        <div className="flex items-center gap-2">
          <ModeTabs mode={mode} onChange={onModeChange} />
          <ModelSelect
            models={models}
            selectedModel={selectedModel}
            onChange={onModelChange}
          />
        </div>
        <p className="text-xs text-subtle">Enter 发送 · Shift+Enter 换行</p>
      </div>
    </div>
  );
}
