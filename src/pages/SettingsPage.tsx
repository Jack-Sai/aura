import { useEffect, useState, type KeyboardEvent } from "react";
import { listen } from "@tauri-apps/api/event";
import { ArrowLeft, Check, ChevronRight, Eye, EyeOff, Plus, RefreshCw, Trash2, Zap } from "lucide-react";
import type { Theme } from "../hooks/useTheme";
import {
  fetchRemoteModels,
  getGlobalRules,
  getProviderStatuses,
  getProviders,
  pullOllamaModel,
  refreshProviderModels,
  setGlobalRules,
  setPinnedModels,
  setProviders,
  type ProviderConfig,
  type ProviderKind,
  type ProviderStatus,
  type RemoteModel,
} from "../lib/api";
import {
  ALL_PRESETS,
  CUSTOM_PRESET_ID,
  presetById,
  providerDisplayName,
} from "../lib/providers";
import { useStore } from "../store";
import Select from "../components/Select";

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

/** 厂商下拉项直接来自预设目录 */
const PRESET_OPTIONS = ALL_PRESETS.map((p) => ({ value: p.id, label: p.label }));

/**
 * 推断某张供应商卡片当前对应的预设 id。
 * 优先用持久化的 preset；旧配置无此字段时按 base_url 反查，
 * 查不到则回退「自定义」。
 */
function inferPreset(p: ProviderConfig): string {
  if (p.preset && presetById(p.preset)) return p.preset;
  const hit = ALL_PRESETS.find(
    (x) => x.baseUrl && x.baseUrl === p.base_url.replace(/\/+$/, ""),
  );
  return hit?.id ?? CUSTOM_PRESET_ID;
}

function isLocalKind(kind: ProviderKind): boolean {
  return kind === "ollama" || kind === "llama_cpp";
}

/** 本地框架地址拆分为 主机 / 端口 / 路径 三段（配置端口号与接口路径）。 */
function splitLocal(base: string): { host: string; port: string; path: string } {
  const rest = base.replace(/^https?:\/\//, "");
  const slash = rest.indexOf("/");
  const authority = slash >= 0 ? rest.slice(0, slash) : rest;
  const path = slash >= 0 ? rest.slice(slash) : "";
  const colon = authority.lastIndexOf(":");
  if (colon > -1) {
    return {
      host: authority.slice(0, colon),
      port: authority.slice(colon + 1),
      path,
    };
  }
  return { host: authority, port: "", path };
}

function joinLocal(host: string, port: string, path: string): string {
  const h = host.trim();
  if (!h) return "";
  const p = /^\d+$/.test(port.trim()) ? `:${port.trim()}` : "";
  let s = path.trim();
  if (s && !s.startsWith("/")) s = `/${s}`;
  return `http://${h}${p}${s}`;
}

const LOCAL_PRESETS: { label: string; host: string; port: string; path: string }[] = [
  { label: "Ollama", host: "127.0.0.1", port: "11434", path: "" },
  { label: "llama.cpp", host: "127.0.0.1", port: "8080", path: "" },
  { label: "LM Studio", host: "127.0.0.1", port: "1234", path: "" },
  { label: "vLLM", host: "127.0.0.1", port: "8000", path: "" },
];

export default function SettingsPage({ onClose, theme, setTheme }: Props) {
  const { models, selectedModel, setModel, refreshModels } = useStore();
  // 已收藏模型按厂商真实分组（provider_name 已由后端解析为自定义名优先）。
  // 用 Map 归并而非相邻聚合，否则同厂商模型不连续时会被拆成多个同名标题。
  const pinnedGroups = (() => {
    const map = new Map<string, typeof models>();
    for (const m of models.filter((x) => x.pinned)) {
      const name = m.provider_name || providerDisplayName(m.provider);
      const bucket = map.get(name);
      if (bucket) bucket.push(m);
      else map.set(name, [m]);
    }
    return [...map].map(([provider, list]) => ({
      provider,
      models: list,
    }));
  })();
  const [section, setSection] = useState<SectionId>("appearance");
  const [rules, setRules] = useState("");
  const [saved, setSaved] = useState(false);
  const [providers, setProvidersState] = useState<ProviderConfig[]>([]);
  const [provSaved, setProvSaved] = useState(false);
  const [provError, setProvError] = useState("");
  const [visibleKeys, setVisibleKeys] = useState<Record<string, boolean>>({});
  const [statuses, setStatuses] = useState<Record<string, ProviderStatus>>({});
  const [probeBusy, setProbeBusy] = useState(false);
  const [pullInputs, setPullInputs] = useState<Record<string, string>>({});
  const [pulls, setPulls] = useState<
    Record<string, { status: string; percent: number | null; failed?: boolean }>
  >({});
  const [pulling, setPulling] = useState<Record<string, boolean>>({});
  const [testingConn, setTestingConn] = useState<Record<string, boolean>>({});
  const [testResults, setTestResults] = useState<
    Record<string, { ok: boolean; message: string }>
  >({});
  const [customModels, setCustomModels] = useState<
    Record<string, Array<{ id: string; label: string; context_limit: number }>>
  >({});
  // 模型目录展开区状态：provider id -> 草稿（勾选的模型 key 集合 + 搜索词）
  const [catalogOpen, setCatalogOpen] = useState<Record<string, boolean>>({});
  const [catalogDraft, setCatalogDraft] = useState<Record<string, string[]>>({});
  const [catalogList, setCatalogList] = useState<
    Record<string, RemoteModel[]>
  >({});
  const [catalogQuery, setCatalogQuery] = useState<Record<string, string>>({});
  const [catalogBusy, setCatalogBusy] = useState<Record<string, boolean>>({});
  const [catalogError, setCatalogError] = useState<Record<string, string>>({});
  const [catalogSaved, setCatalogSaved] = useState<Record<string, boolean>>({});

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
      .then((ps) => {
        setProvidersState(ps);
        setCustomModels({});
      })
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

  function addCustomModel(i: number) {
    const pid = providers[i].id;
    setCustomModels((cm) => ({
      ...cm,
      [pid]: [...(cm[pid] || []), { id: "", label: "", context_limit: 4096 }],
    }));
  }

  function removeCustomModel(i: number, mi: number) {
    const pid = providers[i].id;
    setCustomModels((cm) => ({
      ...cm,
      [pid]: cm[pid]?.filter((_, idx) => idx !== mi) || [],
    }));
  }

  function updateCustomModel(i: number, mi: number, patch: Partial<{ id: string; label: string; context_limit: number }>) {
    const pid = providers[i].id;
    setCustomModels((cm) => ({
      ...cm,
      [pid]: cm[pid]?.map((m, idx) => (idx === mi ? { ...m, ...patch } : m)) || [],
    }));
  }

  async function testConnection(id: string) {
    setTestingConn((t) => ({ ...t, [id]: true }));
    setTestResults((r) => ({ ...r, [id]: { ok: false, message: "测试中..." } }));
    try {
      const list = await getProviderStatuses();
      const st = list.find((s) => s.id === id);
      setTestResults((r) => ({
        ...r,
        [id]: st
          ? { ok: st.ok, message: st.message + (st.latency_ms > 0 ? ` (${st.latency_ms}ms)` : "") }
          : { ok: false, message: "未找到供应商" },
      }));
    } catch (e) {
      setTestResults((r) => ({
        ...r,
        [id]: { ok: false, message: typeof e === "string" ? e : "测试失败" },
      }));
    } finally {
      setTestingConn((t) => ({ ...t, [id]: false }));
    }
  }

  /**
   * 展开/收起模型目录。展开时按**当前卡片草稿配置**拉取远端模型列表——
   * 用户刚切换厂商还没点「保存」，此时后端内存里仍是旧厂商，
   * 若走 refresh_provider_models 无论选哪家都只会拉到 OpenRouter 的模型。
   */
  async function toggleCatalog(index: number) {
    const p = providers[index];
    if (!p) return;
    const opening = !catalogOpen[p.id];
    setCatalogOpen((m) => ({ ...m, [p.id]: opening }));
    if (!opening) return;
    setCatalogError((e) => ({ ...e, [p.id]: "" }));
    setCatalogBusy((b) => ({ ...b, [p.id]: true }));
    try {
      const list = await fetchRemoteModels(p);
      setCatalogList((l) => ({ ...l, [p.id]: list }));
      // 草稿初始 = 该供应商当前已收藏的模型
      setCatalogDraft((d) => ({
        ...d,
        [p.id]: currentPinnedKeys(p.id),
      }));
    } catch (e) {
      const msg = typeof e === "string" ? e : "获取失败";
      setCatalogError((x) => ({
        ...x,
        [p.id]: /不支持|404|未实现|not found|method not allowed/i.test(msg)
          ? `${msg}。该厂商可能不提供模型列表接口，请手动添加模型`
          : msg,
      }));
    } finally {
      setCatalogBusy((b) => ({ ...b, [p.id]: false }));
    }
  }

  function togglePin(id: string, key: string) {
    setCatalogDraft((d) => {
      const cur = d[id] ?? currentPinnedKeys(id);
      return {
        ...d,
        [id]: cur.includes(key) ? cur.filter((k) => k !== key) : [...cur, key],
      };
    });
  }

  /** 该供应商当前已收藏的模型 key（草稿未建立时的回退值） */
  function currentPinnedKeys(id: string): string[] {
    return models.filter((m) => m.provider === id && m.pinned).map((m) => m.key);
  }

  function draftFor(id: string): string[] {
    return catalogDraft[id] ?? currentPinnedKeys(id);
  }

  /**
   * 保存收藏。三步顺序不可颠倒：
   * 1) 先把本地拉取到的模型并入配置（pinned=false）
   * 2) 再保存供应商配置（接口地址/密钥），确保按 id 找得到执行器
   * 3) 最后写入收藏状态
   */
  async function saveCatalog(index: number) {
    const p = providers[index];
    if (!p) return;
    const keys = draftFor(p.id);
    setCatalogBusy((b) => ({ ...b, [p.id]: true }));
    setCatalogError((e) => ({ ...e, [p.id]: "" }));
    try {
      if ((catalogList[p.id] ?? []).length > 0) {
        await refreshProviderModels(p.id);
      }
      await persistProviders();
      await setPinnedModels(p.id, keys);
      await refreshModels();
      setCatalogSaved((s) => ({ ...s, [p.id]: true }));
      window.setTimeout(
        () => setCatalogSaved((s) => ({ ...s, [p.id]: false })),
        1500,
      );
    } catch (e) {
      setCatalogError((x) => ({
        ...x,
        [p.id]: typeof e === "string" ? e : "保存失败",
      }));
    } finally {
      setCatalogBusy((b) => ({ ...b, [p.id]: false }));
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

  /**
   * 切换厂商：自动填入对应的接口地址与端点特例。
   *
   * 仅当当前地址为空、或仍等于上一个厂商的预设地址时才覆盖，
   * 避免抹掉用户手动修改过的地址。
   */
  function applyPreset(index: number, presetId: string) {
    const preset = presetById(presetId);
    const cur = providers[index];
    if (!preset || !cur) return;

    const curPresetId = inferPreset(cur);
    const curPreset = presetById(curPresetId);
    const stillPristine =
      cur.base_url.trim() === "" ||
      (!!curPreset?.baseUrl &&
        cur.base_url.trim().replace(/\/+$/, "") ===
          curPreset.baseUrl.replace(/\/+$/, ""));

    const patch: Partial<ProviderConfig> = {
      preset: presetId,
      kind: preset.kind,
      models_path: preset.modelsPath ?? null,
    };
    if (stillPristine) {
      patch.base_url = preset.baseUrl;
      patch.models_path = preset.modelsPath ?? null;
    }
    // 名称仍是默认值时跟随厂商名，用户改过则保留
    if (!cur.name.trim() || curPreset?.label === cur.name.trim()) {
      patch.name = preset.label;
    }
    // Azure 专有字段：切出 Azure 时清空，切回 Azure 时给默认值
    patch.deployment = preset.kind === "azure" ? (cur.deployment ?? "") : null;
    patch.api_version =
      preset.kind === "azure" ? (cur.api_version ?? "2024-06-01") : null;

    updateProvider(index, patch);
  }

  function addProvider() {
    const n = providers.length + 1;
    setProvidersState([
      ...providers,
      {
        id: `custom${Date.now() % 1000000}`,
        preset: CUSTOM_PRESET_ID,
        kind: "custom",
        name: `自定义供应商 ${n}`,
        base_url: "",
        api_key: "",
        headers: {},
        models_path: null,
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

  /** 持久化供应商配置并刷新模型列表；不重载页面 */
  async function persistProviders(): Promise<void> {
    await setProviders(providers);
    await refreshModels();
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
                          <div className="w-[150px] shrink-0">
                            <Select
                              value={inferPreset(p)}
                              options={PRESET_OPTIONS}
                              onChange={(v) => applyPreset(i, v)}
                              aria-label="模型厂商"
                            />
                          </div>
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
                            onClick={() => testConnection(p.id)}
                            disabled={testingConn[p.id]}
                            aria-label="测试连接"
                            className="shrink-0 p-1 text-subtle transition-colors hover:text-foreground disabled:opacity-60"
                          >
                            <Zap size={14} className={testingConn[p.id] ? "animate-spin" : ""} />
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
                        {testResults[p.id] && (
                          <p className="mt-2 text-xs" style={{ color: testResults[p.id].ok ? "var(--color-success)" : "var(--color-danger)" }}>
                            {testResults[p.id].message}
                          </p>
                        )}

                        <div className="mt-3 grid gap-3 sm:grid-cols-2">
                          <div>
                            <label className="block text-xs text-subtle">
                              {isLocalKind(p.kind) ? "主机 / 端口 / 路径" : "接口地址"}
                            </label>
                            {isLocalKind(p.kind) ? (
                              <>
                                <div className="mt-1 grid grid-cols-2 gap-1.5">
                                  <input
                                    value={splitLocal(p.base_url).host}
                                    onChange={(e) =>
                                      updateProvider(i, {
                                        base_url: joinLocal(
                                          e.target.value,
                                          splitLocal(p.base_url).port,
                                          splitLocal(p.base_url).path,
                                        ),
                                      })
                                    }
                                    placeholder="127.0.0.1"
                                    aria-label="主机"
                                    className="w-full rounded-lg border border-line bg-surface px-2.5 py-1.5 font-mono text-xs outline-none transition-colors placeholder:text-subtle focus:border-brand"
                                  />
                                  <input
                                    value={splitLocal(p.base_url).port}
                                    onChange={(e) =>
                                      updateProvider(i, {
                                        base_url: joinLocal(
                                          splitLocal(p.base_url).host,
                                          e.target.value,
                                          splitLocal(p.base_url).path,
                                        ),
                                      })
                                    }
                                    placeholder="11434"
                                    aria-label="端口"
                                    className="w-full rounded-lg border border-line bg-surface px-2.5 py-1.5 font-mono text-xs outline-none transition-colors placeholder:text-subtle focus:border-brand"
                                  />
                                </div>
                                <input
                                  value={splitLocal(p.base_url).path}
                                  onChange={(e) =>
                                    updateProvider(i, {
                                      base_url: joinLocal(
                                        splitLocal(p.base_url).host,
                                        splitLocal(p.base_url).port,
                                        e.target.value,
                                      ),
                                    })
                                  }
                                  placeholder="路径（可空，如 /v1）"
                                  aria-label="接口路径"
                                  className="mt-1.5 w-full rounded-lg border border-line bg-surface px-2.5 py-1.5 font-mono text-xs outline-none transition-colors placeholder:text-subtle focus:border-brand"
                                />
                                <div className="mt-1.5 flex flex-wrap gap-1.5">
                                  {LOCAL_PRESETS.map((preset) => (
                                    <button
                                      key={preset.label}
                                      type="button"
                                      onClick={() =>
                                        updateProvider(i, {
                                          base_url: joinLocal(
                                            preset.host,
                                            preset.port,
                                            preset.path,
                                          ),
                                        })
                                      }
                                      className="rounded-full border border-line px-2 py-0.5 text-[11px] text-subtle transition-colors hover:text-foreground"
                                    >
                                      {preset.label}
                                    </button>
                                  ))}
                                </div>
                              </>
                            ) : (
                              <input
                                value={p.base_url}
                                onChange={(e) =>
                                  updateProvider(i, { base_url: e.target.value })
                                }
                                placeholder={
                                  presetById(inferPreset(p))?.baseUrl ||
                                  "https://your-endpoint/v1"
                                }
                                className="mt-1 w-full rounded-lg border border-line bg-surface px-2.5 py-1.5 font-mono text-xs outline-none transition-colors placeholder:text-subtle focus:border-brand"
                              />
                            )}
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
                          {p.kind === "custom" && (
                            <div className="sm:col-span-2">
                              <label className="block text-xs text-subtle mb-1">自定义请求头</label>
                              <div className="space-y-1.5">
                                {Object.entries(p.headers).map(([hk, hv]) => (
                                  <div key={hk} className="flex items-center gap-1.5">
                                    <input
                                      value={hk}
                                      onChange={(e) => {
                                        const newHeaders = { ...p.headers };
                                        delete newHeaders[hk];
                                        newHeaders[e.target.value] = hv;
                                        updateProvider(i, { headers: newHeaders });
                                      }}
                                      placeholder="Header 名"
                                      className="flex-1 rounded-lg border border-line bg-surface px-2.5 py-1.5 font-mono text-xs outline-none transition-colors placeholder:text-subtle focus:border-brand"
                                    />
                                    <input
                                      value={String(hv)}
                                      onChange={(e) => {
                                        const newHeaders = { ...p.headers };
                                        newHeaders[hk] = e.target.value;
                                        updateProvider(i, { headers: newHeaders });
                                      }}
                                      placeholder="Header 值"
                                      className="flex-1 rounded-lg border border-line bg-surface px-2.5 py-1.5 font-mono text-xs outline-none transition-colors placeholder:text-subtle focus:border-brand"
                                    />
                                    <button
                                      type="button"
                                      onClick={() => {
                                        const newHeaders = { ...p.headers };
                                        delete newHeaders[hk];
                                        updateProvider(i, { headers: newHeaders });
                                      }}
                                      className="p-1 text-subtle hover:text-danger"
                                    >
                                      <Trash2 size={14} />
                                    </button>
                                  </div>
                                ))}
                                <button
                                  type="button"
                                  onClick={() => {
                                    const newHeaders = { ...p.headers, "X-Custom": "" };
                                    updateProvider(i, { headers: newHeaders });
                                  }}
                                  className="flex items-center gap-1.5 rounded-full border border-dashed border-line px-3 py-1 text-xs text-subtle transition-colors hover:text-foreground"
                                >
                                  <Plus size={12} />
                                  添加 Header
                                </button>
                              </div>
                            </div>
                          )}
                          {p.kind === "custom" && (
                            <div className="sm:col-span-2">
                              <label className="block text-xs text-subtle mb-1">模型映射（{p.id} 专用）</label>
                              <div className="space-y-1.5">
                                {customModels[p.id]?.map((m, mi) => (
                                  <div key={m.id} className="flex items-center gap-1.5">
                                    <input
                                      value={m.id}
                                      onChange={(e) =>
                                        updateCustomModel(i, mi, { id: e.target.value })
                                      }
                                      placeholder="模型 ID（如 gpt-4o）"
                                      className="flex-1 rounded-lg border border-line bg-surface px-2.5 py-1.5 font-mono text-xs outline-none transition-colors placeholder:text-subtle focus:border-brand"
                                    />
                                    <input
                                      value={m.label}
                                      onChange={(e) =>
                                        updateCustomModel(i, mi, { label: e.target.value })
                                      }
                                      placeholder="显示名称"
                                      className="flex-1 rounded-lg border border-line bg-surface px-2.5 py-1.5 text-xs outline-none transition-colors placeholder:text-subtle focus:border-brand"
                                    />
                                    <input
                                      value={m.context_limit}
                                      onChange={(e) =>
                                        updateCustomModel(i, mi, { context_limit: parseInt(e.target.value) || 4096 })
                                      }
                                      type="number"
                                      placeholder="上下文长度"
                                      className="w-24 rounded-lg border border-line bg-surface px-2.5 py-1.5 font-mono text-xs outline-none transition-colors placeholder:text-subtle focus:border-brand"
                                    />
                                    <button
                                      type="button"
                                      onClick={() => removeCustomModel(i, mi)}
                                      className="p-1 text-subtle hover:text-danger"
                                    >
                                      <Trash2 size={14} />
                                    </button>
                                  </div>
                                ))}
                                <button
                                  type="button"
                                  onClick={() => addCustomModel(i)}
                                  className="flex items-center gap-1.5 rounded-full border border-dashed border-line px-3 py-1 text-xs text-subtle transition-colors hover:text-foreground"
                                >
                                  <Plus size={12} />
                                  添加模型
                                </button>
                              </div>
                            </div>
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
                          {/* 模型目录：获取远端列表 → 搜索勾选收藏 */}
                          <div className="sm:col-span-2 mt-1">
                            <button
                              type="button"
                              onClick={() => toggleCatalog(i)}
                              disabled={catalogBusy[p.id]}
                              className="flex w-full items-center justify-between rounded-lg border border-line px-3 py-2 text-xs text-subtle transition-colors hover:bg-bubble hover:text-foreground disabled:opacity-60"
                            >
                              <span className="flex items-center gap-2">
                                {catalogBusy[p.id] ? (
                                  <RefreshCw size={13} className="animate-spin" />
                                ) : (
                                  <ChevronRight
                                    size={13}
                                    className={`transition-transform ${catalogOpen[p.id] ? "rotate-90" : ""}`}
                                  />
                                )}
                                获取模型列表
                              </span>
                              <span>
                                {models.filter(
                                  (m) => m.provider === p.id && m.pinned,
                                ).length > 0 && (
                                  <span className="text-subtle">
                                    已收藏{" "}
                                    {
                                      models.filter(
                                        (m) =>
                                          m.provider === p.id && m.pinned,
                                      ).length
                                    }{" "}
                                    个
                                  </span>
                                )}
                              </span>
                            </button>

                            {catalogOpen[p.id] && (
                              <div className="mt-2 rounded-lg border border-line p-3">
                                {catalogError[p.id] ? (
                                  <p className="text-xs text-danger">
                                    {catalogError[p.id]}
                                  </p>
                                ) : (
                                  <>
                                    <input
                                      value={catalogQuery[p.id] ?? ""}
                                      onChange={(e) =>
                                        setCatalogQuery((q) => ({
                                          ...q,
                                          [p.id]: e.target.value,
                                        }))
                                      }
                                      placeholder="搜索模型…"
                                      aria-label="搜索模型"
                                      className="mb-2 w-full rounded-lg border border-line bg-surface px-2.5 py-1.5 text-xs outline-none transition-colors placeholder:text-subtle focus:border-brand"
                                    />
                                    {(() => {
                                      const q = (
                                        catalogQuery[p.id] ?? ""
                                      )
                                        .trim()
                                        .toLowerCase();
                                      const all = catalogList[p.id] ?? [];
                                      const list = all.filter(
                                        (m) =>
                                          !q ||
                                          m.id.toLowerCase().includes(q) ||
                                          m.label
                                            .toLowerCase()
                                            .includes(q),
                                      );
                                      const draft = draftFor(p.id);
                                      const keyOf = (m: RemoteModel) =>
                                        `${p.id}:${m.id}`;
                                      return (
                                        <>
                                          <div className="max-h-[240px] overflow-y-auto overscroll-contain space-y-0.5">
                                            {all.length === 0 ? (
                                              <p className="py-2 text-center text-xs text-subtle">
                                                该厂商未返回可用模型
                                              </p>
                                            ) : list.length === 0 ? (
                                              <p className="py-2 text-center text-xs text-subtle">
                                                无匹配模型
                                              </p>
                                            ) : (
                                              list.map((m) => {
                                                const checked =
                                                  draft.includes(
                                                    keyOf(m),
                                                  );
                                                return (
                                                  <label
                                                    key={keyOf(m)}
                                                    className="flex cursor-pointer items-center gap-2 rounded-lg px-2 py-1.5 text-xs hover:bg-bubble"
                                                  >
                                                    <input
                                                      type="checkbox"
                                                      checked={checked}
                                                      onChange={() =>
                                                        togglePin(
                                                          p.id,
                                                          keyOf(m),
                                                        )
                                                      }
                                                      className="accent-[var(--color-brand)]"
                                                    />
                                                    <span className="min-w-0 flex-1 truncate">
                                                      {m.label}
                                                      <span className="ml-1.5 font-mono text-[10px] text-subtle">
                                                        {m.id}
                                                      </span>
                                                    </span>
                                                  </label>
                                                );
                                              })
                                            )}
                                          </div>
                                          <div className="mt-2 flex items-center justify-between">
                                            <span className="text-[11px] text-subtle">
                                              已选 {draft.length} 个
                                            </span>
                                            <button
                                              type="button"
                                              onClick={() =>
                                                saveCatalog(i)
                                              }
                                              disabled={catalogBusy[p.id]}
                                              className="rounded-full bg-brand px-3 py-1 text-xs text-white transition-colors enabled:hover:bg-brand-strong disabled:opacity-60"
                                            >
                                              {catalogSaved[p.id]
                                                ? "已保存"
                                                : catalogBusy[p.id]
                                                  ? "保存中…"
                                                  : "保存收藏"}
                                            </button>
                                          </div>
                                        </>
                                      );
                                    })()}
                                  </>
                                )}
                              </div>
                            )}
                          </div>
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
                  <span className="text-sm font-medium">已收藏模型</span>
                  <p className="mt-1 text-xs text-subtle">
                    在上方供应商卡片中点击「获取模型列表」勾选收藏。已收藏模型按厂商分组，
                    点击即可设为当前模型；降级链按厂商分组内顺序排列
                  </p>
                  {pinnedGroups.length === 0 ? (
                    <p className="mt-3 rounded-lg border border-dashed border-line px-3 py-4 text-center text-xs text-subtle">
                      尚未收藏任何模型
                    </p>
                  ) : (
                    <div className="mt-3 space-y-4">
                      {pinnedGroups.map((g) => (
                        <div key={g.provider}>
                          <div className="px-3 pb-1 text-[11px] text-subtle">
                            {g.provider}
                          </div>
                          <ul role="listbox" aria-label={`${g.provider} 模型`} className="space-y-1">
                            {g.models.map((m) => {
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
                                      {isActive && (
                                        <Check size={13} className="text-brand" />
                                      )}
                                    </span>
                                  </button>
                                </li>
                              );
                            })}
                          </ul>
                        </div>
                      ))}
                    </div>
                  )}
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
