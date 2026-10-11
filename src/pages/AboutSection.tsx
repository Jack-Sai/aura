import { useEffect, useState } from "react";
import { openUrl } from "@tauri-apps/plugin-opener";
import { getVersion } from "@tauri-apps/api/app";
import {
  ArrowDownToLine,
  Check,
  ExternalLink,
  RefreshCw,
  TriangleAlert,
} from "lucide-react";
import {
  checkForUpdate,
  downloadAndInstall,
  restartApp,
  type UpdateInfo,
} from "../lib/updater";

const REPO_URL = "https://github.com/Jack-Sai/aura";

type CheckState =
  | { kind: "idle" }
  | { kind: "checking" }
  | { kind: "result"; info: UpdateInfo }
  | { kind: "failed" }
  | { kind: "downloading"; downloaded: number; total: number | null }
  | { kind: "installed" }
  | { kind: "error"; message: string };

/** 字节数转可读体积。 */
function fmtSize(bytes: number): string {
  if (bytes < 1024) return `${bytes} B`;
  if (bytes < 1024 * 1024) return `${(bytes / 1024).toFixed(1)} KB`;
  return `${(bytes / 1024 / 1024).toFixed(1)} MB`;
}

export default function AboutSection() {
  const [version, setVersion] = useState("");
  const [state, setState] = useState<CheckState>({ kind: "idle" });

  useEffect(() => {
    getVersion()
      .then(setVersion)
      .catch(() => setVersion("未知"));
  }, []);

  async function handleCheck() {
    if (!version) return;
    setState({ kind: "checking" });
    const info = await checkForUpdate(version);
    // null 表示检查失败（网络不可达或配置缺失），与「已是最新」必须区分
    setState(info ? { kind: "result", info } : { kind: "failed" });
  }

  async function handleInstall() {
    setState({ kind: "downloading", downloaded: 0, total: null });
    try {
      await downloadAndInstall((downloaded, total) => {
        setState({ kind: "downloading", downloaded, total });
      });
      setState({ kind: "installed" });
      await restartApp();
    } catch (e) {
      setState({ kind: "error", message: String(e) });
    }
  }

  const busy =
    state.kind === "checking" ||
    state.kind === "downloading" ||
    state.kind === "installed";

  return (
    <div className="flex flex-col gap-6">
      <section>
        <h3 className="mb-3 text-sm font-medium">Aura</h3>
        <div className="flex items-center gap-3 rounded-card border border-line bg-surface p-4">
          <img src="/logo.png" alt="Aura" className="h-12 w-12" draggable={false} />
          <div className="min-w-0 flex-1">
            <p className="text-sm font-medium">本地优先的桌面 AI Agent</p>
            <p className="mt-0.5 text-xs text-subtle">
              版本 {version || "—"} · 会话与密钥全部存于本机
            </p>
          </div>
        </div>
      </section>

      <section>
        <h3 className="mb-3 text-sm font-medium">项目</h3>
        <div className="flex flex-col gap-1">
          <button
            type="button"
            onClick={() => openUrl(REPO_URL)}
            className="flex items-center gap-2 rounded-card border border-line bg-surface px-3 py-2.5 text-left text-sm transition-colors hover:bg-bubble"
          >
            <ExternalLink size={14} className="shrink-0 text-subtle" />
            <span className="min-w-0 flex-1 truncate">源代码仓库</span>
            <span className="shrink-0 text-xs text-subtle">{REPO_URL}</span>
          </button>
          <button
            type="button"
            onClick={() => openUrl(`${REPO_URL}/releases`)}
            className="flex items-center gap-2 rounded-card border border-line bg-surface px-3 py-2.5 text-left text-sm transition-colors hover:bg-bubble"
          >
            <ArrowDownToLine size={14} className="shrink-0 text-subtle" />
            <span className="min-w-0 flex-1 truncate">历史版本下载</span>
          </button>
        </div>
      </section>

      <section>
        <h3 className="mb-3 text-sm font-medium">更新</h3>
        <div className="flex flex-col gap-2 rounded-card border border-line bg-surface p-4">
          {state.kind === "result" && !state.info.available && (
            <p className="flex items-center gap-2 text-sm text-subtle">
              <Check size={14} className="text-brand" />
              已是最新版本
            </p>
          )}

          {state.kind === "result" && state.info.available && (
            <>
              <p className="text-sm">
                发现新版本 {state.info.version}
                <span className="ml-2 text-xs text-subtle">
                  当前 {state.info.currentVersion}
                </span>
              </p>
              {state.info.notes && (
                <p className="max-h-32 overflow-y-auto whitespace-pre-wrap text-xs leading-relaxed text-subtle">
                  {state.info.notes}
                </p>
              )}
              <button
                type="button"
                onClick={handleInstall}
                disabled={busy}
                className="mt-1 flex items-center justify-center gap-1.5 rounded-md border border-line bg-bubble py-2 text-sm transition-colors hover:border-brand hover:text-brand disabled:opacity-40"
              >
                <ArrowDownToLine size={14} />
                下载并安装
              </button>
            </>
          )}

          {state.kind === "failed" && (
            <p className="flex items-start gap-2 text-sm text-subtle">
              <TriangleAlert size={14} className="mt-0.5 shrink-0" />
              <span>检查失败，请检查网络连接后重试。</span>
            </p>
          )}

          {state.kind === "downloading" && (
            <div className="flex flex-col gap-1.5">
              <p className="text-sm text-subtle">
                正在下载更新…
                {state.total ? ` ${fmtSize(state.downloaded)} / ${fmtSize(state.total)}` : ""}
              </p>
              <div className="h-1 overflow-hidden rounded-full bg-line">
                <div
                  className="h-full bg-brand transition-all"
                  style={{
                    width: state.total
                      ? `${Math.min(100, (state.downloaded / state.total) * 100)}%`
                      : "30%",
                  }}
                />
              </div>
              <p className="text-xs text-subtle">下载完成后应用会自动重启</p>
            </div>
          )}

          {state.kind === "installed" && (
            <p className="text-sm text-subtle">更新已安装，正在重启…</p>
          )}

          {state.kind === "error" && (
            <p className="flex items-start gap-2 text-sm text-error">
              <TriangleAlert size={14} className="mt-0.5 shrink-0" />
              <span>安装失败：{state.message}</span>
            </p>
          )}

          {state.kind === "idle" && (
            <p className="text-sm text-subtle">
              检查是否有新版本。更新会在下载完成后自动重启应用。
            </p>
          )}

          <button
            type="button"
            onClick={handleCheck}
            disabled={busy || !version}
            className="mt-1 flex items-center justify-center gap-1.5 rounded-md border border-line py-2 text-sm transition-colors hover:bg-bubble disabled:opacity-40"
          >
            <RefreshCw size={14} className={state.kind === "checking" ? "animate-spin" : ""} />
            {state.kind === "checking" ? "检查中…" : "检查更新"}
          </button>
        </div>
      </section>
    </div>
  );
}