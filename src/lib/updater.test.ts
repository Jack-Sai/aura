import { describe, expect, it } from "vitest";

/**
 * 版本号比较。updater 依赖服务端返回的 `latest.json` 判定是否有更新，
 * 这里只测我们自己的格式化与展示逻辑，避免依赖网络。
 */

/** 字节数转可读体积。 */
function fmtSize(bytes: number): string {
  if (bytes < 1024) return `${bytes} B`;
  if (bytes < 1024 * 1024) return `${(bytes / 1024).toFixed(1)} KB`;
  return `${(bytes / 1024 / 1024).toFixed(1)} MB`;
}

describe("fmtSize", () => {
  it("字节", () => {
    expect(fmtSize(512)).toBe("512 B");
  });

  it("千字节", () => {
    expect(fmtSize(2048)).toBe("2.0 KB");
  });

  it("兆字节", () => {
    // 安装包约 45MB，下载进度显示的就是这个量级
    expect(fmtSize(45 * 1024 * 1024)).toBe("45.0 MB");
  });

  it("单位边界", () => {
    expect(fmtSize(1023)).toBe("1023 B");
    expect(fmtSize(1024)).toBe("1.0 KB");
    expect(fmtSize(1024 * 1024)).toBe("1.0 MB");
  });

  it("零值", () => {
    expect(fmtSize(0)).toBe("0 B");
  });
});