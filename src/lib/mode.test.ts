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
  it("每个模式都有标签与说明", () => {
    for (const m of MODES) {
      expect(MODE_LABEL[m]).toBeTruthy();
      expect(MODE_HINT[m]).toBeTruthy();
    }
  });

  /// 标签固定为英文：三个词更紧凑，且避免中英混排的基线错位
  it("标签使用英文", () => {
    expect(MODE_LABEL.chat).toBe("Chat");
    expect(MODE_LABEL.plan).toBe("Plan");
    expect(MODE_LABEL.build).toBe("Build");
    for (const m of MODES) {
      expect(MODE_LABEL[m]).toMatch(/^[A-Za-z]+$/);
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

describe("模式说明文案", () => {
  it("不得声称只读模式可以修改文件", () => {
    // 这类文案一旦与实际权限不符，会让用户误以为能改文件而白跑一趟。
    // 只断言「明确否定了权限」，不去匹配具体措辞——措辞可以调整，
    // 但语义不能漂移。
    for (const m of ["chat", "plan"] as const) {
      const hint = MODE_HINT[m];
      expect(hint).toMatch(/不|禁/);
      expect(hint).toMatch(/修改文件|改动/);
      expect(hint).toMatch(/命令/);
      // 否定词必须落在权限词之前，否则就成了「可以修改文件但不能运行命令」
      expect(hint).not.toMatch(/(?<!不)(?<!禁)可(以)?(修改文件|执行命令|读写)/);
    }
  });

  it("执行模式明确说明可修改与可执行", () => {
    expect(MODE_HINT.build).toMatch(/修改文件/);
    expect(MODE_HINT.build).toMatch(/执行命令/);
  });
});