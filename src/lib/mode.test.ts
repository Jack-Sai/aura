import { describe, expect, it } from "vitest";
import {
  MODES,
  MODE_HINT,
  MODE_LABEL,
  normalizeMode,
  showsTodoPanel,
} from "./mode";

describe("normalizeMode", () => {
  it("接受全部合法模式", () => {
    for (const m of MODES) {
      expect(normalizeMode(m)).toBe(m);
    }
  });

  it("对坏值回落 chat 而不抛错", () => {
    // 存盘数据可能被旧版本或手工改坏，不能因此让输入区不可用
    expect(normalizeMode("bogus")).toBe("chat");
    expect(normalizeMode("")).toBe("chat");
    expect(normalizeMode(null)).toBe("chat");
    expect(normalizeMode(undefined)).toBe("chat");
    expect(normalizeMode(42)).toBe("chat");
  });

  it("不区分大小写之外的变形", () => {
    expect(normalizeMode("PLAN")).toBe("chat");
  });
});

describe("模式元数据完整性", () => {
  it("每个模式都有中文名与说明", () => {
    for (const m of MODES) {
      expect(MODE_LABEL[m]).toBeTruthy();
      expect(MODE_HINT[m]).toBeTruthy();
    }
  });
});

describe("showsTodoPanel", () => {
  it("chat 模式不展示面板", () => {
    expect(showsTodoPanel("chat", 5)).toBe(false);
  });

  it("plan 与 build 在有任务时展示", () => {
    expect(showsTodoPanel("plan", 1)).toBe(true);
    expect(showsTodoPanel("build", 3)).toBe(true);
  });

  it("清单为空时不展示空面板", () => {
    expect(showsTodoPanel("plan", 0)).toBe(false);
    expect(showsTodoPanel("build", 0)).toBe(false);
  });
});