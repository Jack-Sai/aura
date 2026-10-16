import { beforeEach, describe, expect, it, vi } from "vitest";
import type { Block } from "../store";

/**
 * 消息流的 block 排布。
 *
 * 这里测的是一个真实的显示顺序 bug：模型「先说一句」再调工具时，
 * 动作卡片走的是直接 append，不经过打字机缓冲。若打字机随后发现
 * 「末块不是 text」，就会新开一块——那句话被挤到动作卡片**后面**，
 * 与模型实际输出顺序相反。
 */
beforeEach(() => {
  vi.resetModules();
  vi.doUnmock("../store");
});

async function load() {
  return (await import("../store")).appendText;
}

const action = (text: string): Block => ({ kind: "action", text, ok: true });

describe("消息流 block 排布", () => {
  it("连续文本合并到同一个块", async () => {
    const appendText = await load();
    let r = appendText([], 0, "第一句");
    expect(r.blocks).toEqual([{ kind: "text", text: "第一句" }]);
    r = appendText(r.blocks, r.index, "第二句");
    expect(r.blocks).toHaveLength(1);
    expect(r.blocks[0]).toEqual({ kind: "text", text: "第一句第二句" });
  });

  it("动作卡片插在中间时，后续文本不会被合并到它前面", async () => {
    const appendText = await load();

    // 模型先说一句 → 打字机写到 index 0
    let r = appendText([], 0, "先说一句");
    expect(r.index).toBe(0);

    // 动作卡片直接 append 到末尾，并把 textIndex 推到它之后
    // （store 的 insertTailBlock 就是这么做的）
    const blocks = [...r.blocks, action("Read a.rs")];
    const afterAction = blocks.length; // = b.length + 1

    // 模型继续说 → 必须出现在动作卡片**后面**
    r = appendText(blocks, afterAction, "继续说");

    expect(r.blocks).toHaveLength(3);
    expect(r.blocks[0]).toEqual({ kind: "text", text: "先说一句" });
    expect(r.blocks[1]).toEqual(action("Read a.rs"));
    expect(r.blocks[2]).toEqual({ kind: "text", text: "继续说" });
    expect(r.index).toBe(2);
  });

  it("连续多个动作各自开块，文本顺序不打乱", async () => {
    const appendText = await load();

    let blocks: Block[] = appendText([], 0, "开工").blocks;
    let index = 0;

    for (const label of ["A", "B", "C"]) {
      const after = blocks.length + 1;
      blocks = [...blocks, action(label)];
      const r = appendText(blocks, after, `第${label}步之后`);
      blocks = r.blocks;
      index = r.index;
    }

    const texts = blocks.filter((b) => b.kind === "text").map((b) => b.text);
    const actions = blocks.filter((b) => b.kind === "action").map((b) => b.text);
    expect(actions).toEqual(["A", "B", "C"]);
    expect(texts).toEqual(["开工", "第A步之后", "第B步之后", "第C步之后"]);
    // 顺序必须是 开工 A 第A B 第B C 第C
    expect(blocks.map((b) => b.kind === "text" ? b.text : `<${b.text}>`)).toEqual([
      "开工",
      "<A>",
      "第A步之后",
      "<B>",
      "第B步之后",
      "<C>",
      "第C步之后",
    ]);
    expect(index).toBe(6);
  });

  it("下标越界时退化为末尾追加而不是崩溃", async () => {
    const appendText = await load();
    // blocks 被外部替换过，textIndex 可能指向不存在的位置
    const r = appendText([{ kind: "text", text: "已有" }], 99, "新文本");
    expect(r.blocks).toHaveLength(2);
    expect(r.blocks[1]).toEqual({ kind: "text", text: "新文本" });
    expect(r.index).toBe(1);
  });

  it("下标指向非文本块时不覆盖它", async () => {
    const appendText = await load();
    // textIndex 误指向 action：必须新开块，而不是把正文塞进动作卡片
    const r = appendText([{ kind: "text", text: "前面" }, action("Act")], 1, "正文");
    expect(r.blocks[1]).toEqual(action("Act"));
    expect(r.blocks[2]).toEqual({ kind: "text", text: "正文" });
  });
});