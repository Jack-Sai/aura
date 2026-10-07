import { useEffect, useState, type KeyboardEvent } from "react";
import { listen } from "@tauri-apps/api/event";
import { ArrowLeft, Check, Eye, EyeOff, Plus, RefreshCw, Trash2 } from "lucide-react";
import type { Theme } from "../hooks/useTheme";
import {
  getGlobalRules,
  getProviderStatuses,
  getProviders,
  pullOllamaModel,
  refreshProviderModels,
  setGlobalRules,
  setProviders,
  type ProviderConfig,
  type ProviderKind,
  type ProviderStatus,
} from "../lib/api";
import { useStore } from "../store";

interface Props {
  onClose: () => void;
  theme: Theme;
  setTheme: (theme: Theme) => void;
}

const SECTIONS = [
  { id: "appearance", label: "外观" },
  { id: "model", label: "模型服务" },
  { id: "general", label: "通用" },
] as const;

type SectionId = (typeof SECTIONS)[number]["id"];

const KIND_OPTIONS: { value: ProviderKind; label: string }[] = [
  { value: "openrouter", label: "OpenRouter" },
  { value: "openai", label: "OpenAI" },
  { value: "azure", label: "Azure OpenAI" },
  { value: "ollama", label: "Ollama" },
  { value: "llama_cpp", label: "llama.cpp" },
  { value: "custom", label: "自定义" },
];

function basePlaceholder(kind: ProviderKind): string {
  switch (kind) {
    case "openrouter":
      return "https://openrouter.ai/api/v1";
    case "openai":
      return "https://api.openai.com/v1";
    case "azure":
      return "https://your-resource.openai.azure.com";
    case "ollama":
      return "http://localhost:11434";
    case "llama_cpp":
      return "http://127.0.0.1:8080";
    default:
      return "http://localhost:8000/v1";
  }
}

export default function SettingsPage({ onClose, theme, setTheme }: Props) {
  const { models, selectedModel, setModel, refreshModels } = useStore();
  const [section, setSection] = useState<SectionId>("appearance");
  const [rules, setRules] = useState("");
  const [saved, setSaved] = useState(false);
  const [providers, setProvidersState] = useState<ProviderConfig[]>([]);
  const [provSaved, setProvSaved] = useState(false);
  const [provError, setProvError] = useState("");
  const [visibleKeys, setVisibleKeys] = useState<Record<string, boolean>>({});
  const [statuses, setStatuses] = useState<Record<string, ProviderStatus>>({});
  const [probeBusy, setProbeBusy] = useState(false);
  const [refreshNote, setRefreshNote] = useState<Record<string, string>>({});
  const [pullInputs, setPullInputs] = useState<Record<string, string>>({});
  const [pulls, setPulls] = useState<
    Record<string, { status: string; percent: number | null; failed?: boolean }>
  >({});
  const [pulling, setPulling] = useState<Record<string, boolean>>({});

  useEffect(() => {
    const un = listen<{
      provider: string;
      status: string;
      percent: number | null;
    }>("ollama_pull", (e) => {
      const { provider, status, percent } = e.payload;
      setPulls((prev) => ({
        ...prev,
        [provider]: { status, percent, failed: prev[provider]?.failed },
      }));
    });
    return () => {
      un.then((f) => f());
    };
  }, []);

  useEffect(() => {
    getGlobalRules()
      .then(setRules)
      .catch(() => {});
    getProviders()
      .then(setProvidersState)
      .catch(() => {});
  }, []);

  useEffect(() => {
    if (section !== "model") return;
    let alive = true;
    setProbeBusy(true);
    getProviderStatuses()
      .then((list) => {
        if (!alive) return;
        setStatuses(Object.fromEntries(list.map((s) => [s.id, s])));
      })
      .catch(() => {})
      .finally(() => alive && setProbeBusy(false));
    return () => {
      alive = false;
    };
  }, [section]);

  async function probeNow() {
    setProbeBusy(true);
    try {
      const list = await getProviderStatuses();
      setStatuses(Object.fromEntries(list.map((s) => [s.id, s])));
    } catch {
      /* 探测失败保持原状态 */
    } finally {
      setProbeBusy(false);
    }
  }

  async function refreshModelsOf(id: string) {
    setRefreshNote((n) => ({ ...n, [id]: "刷新中…" }));
    try {
      const added = await refreshProviderModels(id);
      setRefreshNote((n) => ({
        ...n,
        [id]: added > 0 ? `新增 ${added} 个模型` : "无新增模型",
      }));
      if (added > 0) await refreshModels();
    } catch (e) {
      setRefreshNote((n) => ({
        ...n,
        [id]: typeof e === "string" ? e : "刷新失败",
      }));
    }
  }

  async function startPull(id: string) {
    const model = (pullInputs[id] ?? "").trim();
    if (!model || pulling[id]) return;
    setPulling((p) => ({ ...p, [id]: true }));
    setPulls((p) => ({
      ...p,
      [id]: { status: "连接 Ollama…", percent: null },
    }));
    try {
      const added = await pullOllamaModel(id, model);
      setPulls((p) => ({
        ...p,
        [id]: { status: `完成，新增 ${added} 个模型`, percent: 100 },
      }));
      if (added > 0) await refreshModels();
    } catch (e) {
      setPulls((p) => ({
        ...p,
        [id]: {
          status: typeof e === "string" ? e : "拉取失败",
          percent: null,
          failed: true,
        },
      }));
    } finally {
      setPulling((p) => ({ ...p, [id]: false }));
    }
  }

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

  function updateProvider(index: number, patch: Partial<ProviderConfig>) {
    setProvidersState((list) =>
      list.map((p, i) => (i === index ? { ...p, ...patch } : p)),
    );
  }

  function addProvider() {
    const n = providers.length + 1;
    setProvidersState([
      ...providers,
      {
        id: `custom${Date.now() % 1000000}`,
        kind: "custom",
        name: `自定义供应商 ${n}`,
        base_url: basePlaceholder("custom"),
        api_key: "",
        headers: {},
        deployment: null,
        api_version: null,
        enabled: true,
      },
    ]);
  }

  function removeProvider(id: string) {
    if (providers.length <= 1) {
      setProvError("至少保留一个模型供应商");
      return;
    }
    setProvError("");
    setProvidersState(providers.filter((p) => p.id !== id));
  }

  async function saveProviders() {
    setProvError("");
    try {
      await setProviders(providers);
      setProvSaved(true);
      // 配置变更可能影响模型列表与选中项，稍后重载界面以刷新
      window.setTimeout(() => window.location.reload(), 600);
    } catch (e) {
      setProvError(typeof e === "string" ? e : "保存失败");
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
      <div className="mx-auto w-full max-w-4xl px-8 py-8 xl:max-w-5xl">
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

        <div className="mt-6 flex gap-6">
          <nav className="w-36 shrink-0">
            <ul className="space-y-0.5">
              {SECTIONS.map((s) => (
                <li key={s.id}>
                  <button
                    type="button"
                    onClick={() => setSection(s.id)}
                    aria-current={section === s.id ? "page" : undefined}
                    className={`w-full rounded-tag px-3 py-1.5 text-left text-sm transition-colors ${
                      section === s.id
                        ? "bg-bubble font-medium text-foreground"
                        : "text-subtle hover:bg-bubble hover:text-foreground"
                    }`}
                  >
                    {s.label}
                  </button>
                </li>
              ))}
            </ul>
          </nav>

          <div className="min-w-0 flex-1">
            {section === "appearance" && (
              <section className="rounded-card border border-line p-6">
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
            )}

            {section === "model" && (
              <section className="rounded-card border border-line p-6">
                <h2 className="eyebrow">模型服务</h2>
                <div className="mt-4 flex items-center justify-between">
                  <span className="flex items-center gap-2 text-sm font-medium">
                    模型供应商
                    <button
                      type="button"
                      onClick={probeNow}
                      disabled={probeBusy}
                      className="flex items-center gap-1.5 rounded-full border border-line px-3 py-1 text-xs text-subtle transition-colors hover:text-foreground disabled:opacity-60"
                    >
                      <RefreshCw
                        size={12}
                        className={probeBusy ? "animate-spin" : ""}
                      />
                      {probeBusy ? "检测中…" : "检测连通性"}
                    </button>
                  </span>
                  <button
                    type="button"
                    onClick={addProvider}
                    className="flex items-center gap-1.5 rounded-full border border-line px-3 py-1 text-xs text-subtle transition-colors hover:text-foreground"
                  >
                    <Plus size={12} />
                    添加供应商
                  </button>
                </div>
                <p className="mt-1 text-xs text-subtle">
                  密钥仅保存在本地数据库；模型列表顺序即降级顺序
                </p>

                <div className="mt-3 space-y-3">
                  {providers.map((p, i) => {
                    const showKey = visibleKeys[p.id];
                    const st = statuses[p.id];
                    const dot = st?.ok
                      ? "bg-success"
                      : st
                        ? "bg-danger"
                        : "bg-line";
                    return (
                      <div
                        key={p.id}
                        className="rounded-lg border border-line bg-surface/60 p-4"
                      >
                        <div className="flex flex-wrap items-center gap-2">
                          <span
                            title={
                              st
                                ? `${st.message}${
                                    st.latency_ms > 0
                                      ? `（${st.latency_ms}ms）`
                                      : ""
                                  }`
                                : "尚未检测"
                            }
                            className={`h-2.5 w-2.5 shrink-0 rounded-full ${dot}`}
                          />
                          <select
                            value={p.kind}
                            onChange={(e) =>
                              updateProvider(i, {
                                kind: e.target.value as ProviderKind,
                              })
                            }
                            aria-label="供应商类型"
                            className="rounded-full border border-line bg-surface px-2.5 py-1 text-xs text-subtle outline-none transition-colors focus:border-brand"
                          >
                            {KIND_OPTIONS.map((o) => (
                              <option key={o.value} value={o.value}>
                                {o.label}
                              </option>
                            ))}
                          </select>
                          <input
                            value={p.name}
                            onChange={(e) =>
                              updateProvider(i, { name: e.target.value })
                            }
                            aria-label="供应商名称"
                            className="min-w-0 flex-1 rounded-lg border border-line bg-surface px-2.5 py-1 text-sm outline-none transition-colors focus:border-brand"
                          />
                          <button
                            type="button"
                            role="switch"
                            aria-checked={p.enabled}
                            aria-label={p.enabled ? "禁用供应商" : "启用供应商"}
                            onClick={() =>
                              updateProvider(i, { enabled: !p.enabled })
                            }
                            className={`relative h-5 w-9 shrink-0 rounded-full transition-colors ${
                              p.enabled ? "bg-brand" : "bg-line"
                            }`}
                          >
                            <span
                              className={`absolute top-0.5 h-4 w-4 rounded-full bg-white transition-all ${
                                p.enabled ? "left-4" : "left-0.5"
                              }`}
                            />
                          </button>
                          <button
                            type="button"
                            onClick={() => refreshModelsOf(p.id)}
                            aria-label="刷新模型列表"
                            className="shrink-0 p-1 text-subtle transition-colors hover:text-foreground"
                          >
                            <RefreshCw size={14} />
                          </button>
                          <button
                            type="button"
                            onClick={() => removeProvider(p.id)}
                            aria-label="删除供应商"
                            className="shrink-0 p-1 text-subtle transition-colors hover:text-danger"
                          >
                            <Trash2 size={14} />
                          </button>
                        </div>
                        {refreshNote[p.id] && (
                          <p className="mt-2 text-xs text-subtle">
                            {refreshNote[p.id]}
                          </p>
                        )}

                        <div className="mt-3 grid gap-3 sm:grid-cols-2">
                          <div>
                            <label className="block text-xs text-subtle">
                              接口地址
                            </label>
                            <input
                              value={p.base_url}
                              onChange={(e) =>
                                updateProvider(i, { base_url: e.target.value })
                              }
                              placeholder={basePlaceholder(p.kind)}
                              className="mt-1 w-full rounded-lg border border-line bg-surface px-2.5 py-1.5 font-mono text-xs outline-none transition-colors placeholder:text-subtle focus:border-brand"
                            />
                          </div>
                          <div>
                            <label className="block text-xs text-subtle">
                              API Key
                            </label>
                            <div className="relative mt-1">
                              <input
                                type={showKey ? "text" : "password"}
                                value={p.api_key}
                                onChange={(e) =>
                                  updateProvider(i, { api_key: e.target.value })
                                }
                                placeholder={
                                  p.kind === "ollama" || p.kind === "llama_cpp"
                                    ? "本地服务通常无需密钥"
                                    : "sk-…"
                                }
                                className="w-full rounded-lg border border-line bg-surface px-2.5 py-1.5 pr-8 font-mono text-xs outline-none transition-colors placeholder:text-subtle focus:border-brand"
                              />
                              <button
                                type="button"
                                onClick={() =>
                                  setVisibleKeys((s) => ({
                                    ...s,
                                    [p.id]: !s[p.id],
                                  }))
                                }
                                aria-label={showKey ? "隐藏密钥" : "显示密钥"}
                                className="absolute right-2 top-1/2 -translate-y-1/2 text-subtle transition-colors hover:text-foreground"
                              >
                                {showKey ? <EyeOff size={13} /> : <Eye size={13} />}
                              </button>
                            </div>
                          </div>
                          {p.kind === "azure" && (
                            <>
                              <div>
                                <label className="block text-xs text-subtle">
                                  Deployment
                                </label>
                                <input
                                  value={p.deployment ?? ""}
                                  onChange={(e) =>
                                    updateProvider(i, {
                                      deployment: e.target.value || null,
                                    })
                                  }
                                  placeholder="gpt-4o"
                                  className="mt-1 w-full rounded-lg border border-line bg-surface px-2.5 py-1.5 font-mono text-xs outline-none transition-colors placeholder:text-subtle focus:border-brand"
                                />
                              </div>
                              <div>
                                <label className="block text-xs text-subtle">
                                  API Version
                                </label>
                                <input
                                  value={p.api_version ?? ""}
                                  onChange={(e) =>
                                    updateProvider(i, {
                                      api_version: e.target.value || null,
                                    })
                                  }
                                  placeholder="2024-06-01"
                                  className="mt-1 w-full rounded-lg border border-line bg-surface px-2.5 py-1.5 font-mono text-xs outline-none transition-colors placeholder:text-subtle focus:border-brand"
                                />
                              </div>
                            </>
                          )}
                          {p.kind === "ollama" && (
                            <div className="sm:col-span-2 rounded-lg border border-dashed border-line p-3">
                              <div className="flex items-center gap-2">
                                <input
                                  value={pullInputs[p.id] ?? ""}
                                  onChange={(e) =>
                                    setPullInputs((m) => ({
                                      ...m,
                                      [p.id]: e.target.value,
                                    }))
                                  }
                                  onKeyDown={(e) => {
                                    if (e.key === "Enter") startPull(p.id);
                                  }}
                                  placeholder="qwen3:0.6b"
                                  aria-label="要拉取的模型名"
                                  className="min-w-0 flex-1 rounded-lg border border-line bg-surface px-2.5 py-1.5 font-mono text-xs outline-none transition-colors placeholder:text-subtle focus:border-brand"
                                />
                                <button
                                  type="button"
                                  onClick={() => startPull(p.id)}
                                  disabled={pulling[p.id]}
                                  className="shrink-0 rounded-full bg-brand px-3 py-1.5 text-xs text-white transition-colors enabled:hover:bg-brand-strong disabled:opacity-60"
                                >
                                  {pulling[p.id] ? "拉取中…" : "拉取模型"}
                                </button>
                              </div>
                              {pulls[p.id] && (
                                <div className="mt-2">
                                  <div className="h-1.5 w-full overflow-hidden rounded-full bg-line">
                                    <div
                                      className={`h-full rounded-full transition-all ${
                                        pulls[p.id].failed
                                          ? "bg-danger"
                                          : "bg-brand"
                                      } ${
                                        pulls[p.id].percent == null
                                          ? "w-1/3 animate-pulse"
                                          : ""
                                      }`}
                                      style={
                                        pulls[p.id].percent != null
                                          ? {
                                              width: `${pulls[p.id].percent}%`,
                                            }
                                          : undefined
                                      }
                                    />
                                  </div>
                                  <p className="mt-1 text-xs text-subtle">
                                    {pulls[p.id].status}
                                    {pulls[p.id].percent != null &&
                                      pulls[p.id].status !== "done" &&
                                      ` ${pulls[p.id].percent}%`}
                                  </p>
                                </div>
                              )}
                            </div>
                          )}
                        </div>
                      </div>
                    );
                  })}
                </div>

                {provError && (
                  <p className="mt-3 text-xs text-danger">{provError}</p>
                )}

                <div className="mt-4 flex justify-end">
                  <button
                    type="button"
                    onClick={saveProviders}
                    className="rounded-full bg-brand px-4 py-1.5 text-sm text-white transition-colors enabled:hover:bg-brand-strong"
                  >
                    {provSaved ? "已保存" : "保存"}
                  </button>
                </div>

                <div className="mt-6 border-t border-line pt-5">
                  <span className="text-sm font-medium">模型列表</span>
                  <p className="mt-1 text-xs text-subtle">
                    按列表顺序作为降级链：主模型 429 限流或故障时自动切换到下一个可用模型
                  </p>
                  <ul role="listbox" aria-label="模型列表" className="mt-3 space-y-1">
                    {models.map((m, i) => {
                      const isActive = m.key === selectedModel;
                      return (
                        <li key={m.key}>
                          <button
                            type="button"
                            role="option"
                            aria-selected={isActive}
                            onClick={() => {
                              if (!isActive) setModel(m.key);
                            }}
                            className={`flex w-full items-center gap-3 rounded-lg px-3 py-2 text-left text-sm transition-colors ${
                              isActive ? "bg-bubble" : "hover:bg-bubble"
                            }`}
                          >
                            <span className="w-5 shrink-0 text-xs text-subtle">
                              {i + 1}
                            </span>
                            <span className="min-w-0 flex-1 truncate">
                              {m.label}
                              <span className="ml-2 font-mono text-[11px] text-subtle">
                                {m.id}
                              </span>
                            </span>
                            <span className="shrink-0 rounded-full border border-line bg-surface px-1.5 py-0.5 text-[10px] text-subtle">
                              {Math.round(m.context_limit / 1024)}K
                            </span>
                            <span className="flex w-4 shrink-0 justify-end">
                              {isActive && <Check size={13} className="text-brand" />}
                            </span>
                          </button>
                        </li>
                      );
                    })}
                  </ul>
                </div>
              </section>
            )}

            {section === "general" && (
              <section className="rounded-card border border-line p-6">
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
            )}
          </div>
        </div>
      </div>
    </div>
  );
}
