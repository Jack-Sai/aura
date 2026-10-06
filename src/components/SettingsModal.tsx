import { useEffect, useState, type KeyboardEvent } from "react";
import { X } from "lucide-react";
import { getGlobalRules, setGlobalRules } from "../lib/api";

interface Props {
  onClose: () => void;
}

export default function SettingsModal({ onClose }: Props) {
  const [rules, setRules] = useState("");
  const [saved, setSaved] = useState(false);

  useEffect(() => {
    getGlobalRules()
      .then(setRules)
      .catch(() => {});
  }, []);

  useEffect(() => {
    function onKey(e: globalThis.KeyboardEvent) {
      if (e.key === "Escape") onClose();
    }
    window.addEventListener("keydown", onKey);
    return () => window.removeEventListener("keydown", onKey);
  }, [onClose]);

  async function save() {
    try {
      await setGlobalRules(rules);
      setSaved(true);
      window.setTimeout(() => setSaved(false), 1500);
    } catch {
      /* 保存失败保持面板打开 */
    }
  }

  function handleKeyDown(e: KeyboardEvent<HTMLTextAreaElement>) {
    if ((e.metaKey || e.ctrlKey) && e.key === "Enter") {
      e.preventDefault();
      save();
    }
  }

  return (
    <div
      className="fixed inset-0 z-50 flex items-center justify-center bg-black/40 p-6"
      onClick={onClose}
      role="presentation"
    >
      <div
        className="w-full max-w-lg rounded-card border border-line bg-surface p-6 shadow-xl"
        onClick={(e) => e.stopPropagation()}
        role="dialog"
        aria-modal="true"
        aria-label="设置"
      >
        <div className="flex items-center justify-between">
          <h2 className="text-lg font-semibold tracking-[-0.02em]">设置</h2>
          <button
            type="button"
            onClick={onClose}
            aria-label="关闭设置"
            className="rounded-full p-1.5 text-subtle transition-colors hover:bg-bubble hover:text-foreground"
          >
            <X size={16} />
          </button>
        </div>

        <label
          htmlFor="global-rules"
          className="mt-5 block text-sm font-medium"
        >
          全局规则
        </label>
        <p className="mt-1 text-xs text-subtle">
          每次请求自动附加到系统提示词，例如：使用中文回复、代码及时提交
        </p>
        <textarea
          id="global-rules"
          value={rules}
          onChange={(e) => setRules(e.target.value)}
          onKeyDown={handleKeyDown}
          rows={6}
          placeholder="使用中文回复；代码及时提交…"
          className="mt-2 w-full resize-y rounded-card border border-line bg-surface px-3 py-2.5 text-sm leading-relaxed outline-none transition-colors placeholder:text-subtle focus:border-brand"
        />

        <div className="mt-4 flex justify-end gap-2">
          <button
            type="button"
            onClick={onClose}
            className="rounded-full border border-line px-4 py-1.5 text-sm text-subtle transition-colors hover:bg-bubble hover:text-foreground"
          >
            取消
          </button>
          <button
            type="button"
            onClick={save}
            className="rounded-full bg-brand px-4 py-1.5 text-sm text-white transition-colors enabled:hover:bg-brand-strong"
          >
            {saved ? "已保存" : "保存"}
          </button>
        </div>
      </div>
    </div>
  );
}
