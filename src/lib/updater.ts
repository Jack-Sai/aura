import { check } from "@tauri-apps/plugin-updater";
import { relaunch } from "@tauri-apps/plugin-process";

export interface UpdateInfo {
  available: boolean;
  version: string;
  currentVersion: string;
  notes?: string;
  date?: string;
}

/**
 * 检查是否有新版本。
 *
 * 返回 `null` 表示检查失败（网络不可达、配置文件缺失、签名校验未通过）。
 * 调用方应把它与「已是最新」区分开——前者要提示用户可稍后重试，
 * 后者是正常的成功路径。
 */
export async function checkForUpdate(
  currentVersion: string,
): Promise<UpdateInfo | null> {
  try {
    const update = await check();
    if (!update) {
      return { available: false, version: currentVersion, currentVersion };
    }
    return {
      available: true,
      version: update.version,
      currentVersion,
      notes: update.body ?? undefined,
      date: update.date ?? undefined,
    };
  } catch {
    return null;
  }
}

/**
 * 下载并安装更新。
 *
 * Tauri 的安装分两步：先 `downloadAndInstall` 把包下载并解压到
 * 暂存区，再 `relaunch` 重启应用让新版本生效。缺少第二步的话
 * 包会一直躺在暂存区，用户看不到任何变化。
 *
 * 调用方需自行展示下载进度——大文件下载可能持续数十秒。
 */
export async function downloadAndInstall(
  onProgress?: (downloaded: number, total: number | null) => void,
): Promise<void> {
  // check() 返回 null 表示服务端没有提供比当前版本更新的包，
  // 类型上必须收窄后再用，否则下面 .downloadAndInstall 会崩
  const update = await check();
  if (!update) throw new Error("没有可安装的更新");

  let downloaded = 0;
  let total: number | null = null;
  await update.downloadAndInstall((event) => {
    switch (event.event) {
      case "Started":
        total = event.data.contentLength ?? null;
        onProgress?.(0, total);
        break;
      case "Progress":
        downloaded += event.data.chunkLength;
        onProgress?.(downloaded, total);
        break;
      case "Finished":
        onProgress?.(total ?? downloaded, total);
        break;
    }
  });
}

/** 安装完成后重启应用。 */
export async function restartApp(): Promise<void> {
  await relaunch();
}