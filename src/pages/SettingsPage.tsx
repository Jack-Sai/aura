import { useEffect, useState, type KeyboardEvent } from "react";
import { ArrowLeft } from "lucide-react";
import { getGlobalRules, setGlobalRules } from "../lib/api";

interface Props {
  onClose: () => void;
}

export default function SettingsPage({ onClose }: Props) {
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

  async function saveRules() {
    try {
      await setGlobalRules(rules);
      setSaved(true);
      window.setTimeout(() => setSaved(false), 1500);
    } catch {
      /* 保存失败保持页面状态 */
    }
  }

  function handleKeyDown(e: KeyboardEvent<HTMLTextAreaElement>) {
    if ((e.metaKey || e.ctrlKey) && e.key === "Enter") {
      e.preventDefault();
      saveRules();
    }
  }

  return (
    <div className="min-h-full bg-surface">
      <div className="mx-auto w-full max-w-2xl px-8 py-8">
        <button
          type="button"
          onClick={onClose}
          className="flex items-center gap-1.5 text-sm text-subtle transition-colors hover:text-foreground"
        >
          <ArrowLeft size={15} />
          返回对话
        </button>

        <h1 className="mt-5 text-2xl font-semibold tracking-[-0.03em]">
          设置
        </h1>

        <section className="mt-6 rounded-card border border-line p-6">
          <h2 className="text-sm font-semibold">通用</h2>
          <label
            htmlFor="global-rules"
            className="mt-4 block text-sm font-medium"
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
          <div className="mt-4 flex justify-end">
            <button
              type="button"
              onClick={saveRules}
              className="rounded-full bg-brand px-4 py-1.5 text-sm text-white transition-colors enabled:hover:bg-brand-strong"
            >
              {saved ? "已保存" : "保存"}
            </button>
          </div>
        </section>
      </div>
    </div>
  );
}
