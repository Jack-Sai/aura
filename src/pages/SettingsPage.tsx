import { useEffect, useState, type KeyboardEvent } from "react";
import { ArrowLeft, Eye, EyeOff } from "lucide-react";
import type { Theme } from "../hooks/useTheme";
import { getApiConfig, getGlobalRules, setApiConfig, setGlobalRules } from "../lib/api";

interface Props {
  onClose: () => void;
  theme: Theme;
  setTheme: (theme: Theme) => void;
}

export default function SettingsPage({ onClose, theme, setTheme }: Props) {
  const [rules, setRules] = useState("");
  const [saved, setSaved] = useState(false);
  const [apiKey, setApiKey] = useState("");
  const [showKey, setShowKey] = useState(false);
  const [keySaved, setKeySaved] = useState(false);

  useEffect(() => {
    getGlobalRules()
      .then(setRules)
      .catch(() => {});
    getApiConfig()
      .then((c) => setApiKey(c.api_key))
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

  async function saveApiKey() {
    try {
      await setApiConfig("openrouter", apiKey);
      setKeySaved(true);
      window.setTimeout(() => setKeySaved(false), 1500);
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
          className="flex items-center gap-2 text-sm text-subtle transition-colors hover:text-foreground"
        >
          <ArrowLeft size={15} />
          返回对话
        </button>

        <h1 className="mt-5 text-2xl font-semibold leading-tight tracking-[-0.03em]">
          设置
        </h1>

        <section className="mt-6 rounded-card border border-line p-6">
          <h2 className="eyebrow">外观</h2>
          <div className="mt-4 flex items-center justify-between">
            <span className="text-sm font-medium">主题</span>
            <div className="flex rounded-full border border-line p-0.5">
              <button
                type="button"
                onClick={() => setTheme("light")}
                className={`rounded-full px-4 py-1 text-sm transition-colors ${
                  theme === "light"
                    ? "bg-brand text-white"
                    : "text-subtle hover:text-foreground"
                }`}
              >
                亮色
              </button>
              <button
                type="button"
                onClick={() => setTheme("dark")}
                className={`rounded-full px-4 py-1 text-sm transition-colors ${
                  theme === "dark"
                    ? "bg-brand text-white"
                    : "text-subtle hover:text-foreground"
                }`}
              >
                暗色
              </button>
            </div>
          </div>
        </section>

        <section className="mt-6 rounded-card border border-line p-6">
          <h2 className="eyebrow">模型服务</h2>
          <div className="mt-4 flex items-center justify-between">
            <span className="text-sm font-medium">提供商</span>
            <div className="relative">
              <select
                defaultValue="openrouter"
                disabled
                aria-label="提供商"
                className="appearance-none rounded-full border border-line bg-surface py-1 pl-3 pr-7 text-sm text-subtle outline-none disabled:opacity-60"
              >
                <option value="openrouter">OpenRouter</option>
              </select>
              <span className="pointer-events-none absolute right-2.5 top-1/2 -translate-y-1/2 text-subtle">
                ▾
              </span>
            </div>
          </div>

          <label htmlFor="api-key" className="mt-5 block text-sm font-medium">
            API Key
          </label>
          <p className="mt-1 text-xs text-subtle">
            密钥仅保存在本地数据库，也可通过环境变量 OPENROUTER_API_KEY 提供
          </p>
          <div className="relative mt-2">
            <input
              id="api-key"
              type={showKey ? "text" : "password"}
              value={apiKey}
              onChange={(e) => setApiKey(e.target.value)}
              onKeyDown={(e) => {
                if (e.key === "Enter") saveApiKey();
              }}
              placeholder="sk-or-…"
              className="w-full rounded-lg border border-line bg-surface px-3 py-3 pr-10 text-sm outline-none transition-colors placeholder:text-subtle focus:border-brand"
            />
            <button
              type="button"
              onClick={() => setShowKey((v) => !v)}
              aria-label={showKey ? "隐藏密钥" : "显示密钥"}
              className="absolute right-2.5 top-1/2 -translate-y-1/2 text-subtle transition-colors hover:text-foreground"
            >
              {showKey ? <EyeOff size={15} /> : <Eye size={15} />}
            </button>
          </div>
          <div className="mt-4 flex justify-end">
            <button
              type="button"
              onClick={saveApiKey}
              className="rounded-full bg-brand px-4 py-1.5 text-sm text-white transition-colors enabled:hover:bg-brand-strong"
            >
              {keySaved ? "已保存" : "保存"}
            </button>
          </div>
        </section>

        <section className="mt-6 rounded-card border border-line p-6">
          <h2 className="eyebrow">通用</h2>
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
            className="mt-2 w-full resize-y rounded-lg border border-line bg-surface px-3 py-3 text-sm leading-relaxed outline-none transition-colors placeholder:text-subtle focus:border-brand"
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
