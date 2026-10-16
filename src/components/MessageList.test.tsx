import { render } from "@testing-library/react";
import { beforeEach, describe, expect, it, vi } from "vitest";

beforeEach(() => {
  vi.resetModules();
  vi.doUnmock("../store");
  vi.doUnmock("./Markdown");
});

function msg(id: string, text: string) {
  return { id, role: "assistant" as const, blocks: [{ kind: "text" as const, text }] };
}

describe("MessageList 渲染开销", () => {
  /**
   * 打字机每 30ms 触发一次 setState，若历史消息跟着重渲染，
   * 就会连带把其中每个 Markdown 重新解析一遍（remark + 高亮 + KaTeX）。
   * 长对话积累到几百个 block 后，这会让流式输出明显卡顿——
   * 且**不会报错**，只会越来越慢。
   *
   * memo 能拦住的前提是 store 侧对未参与更新的消息保持对象引用不变。
   * 一旦有人把 appendBlocks 改成深拷贝或全量重建，memo 会静默失效，
   * 所以在这里钉住。
   */
  it("打字机推进时只重新渲染正在接收流量的那一条消息", async () => {
    const renderCount: Record<string, number> = {};
    vi.doMock("./Markdown", () => ({
      default: ({ content }: { content: string }) => {
        renderCount[content] = (renderCount[content] ?? 0) + 1;
        return <div data-testid="md">{content}</div>;
      },
    }));

    const history = msg("m1", "已经写完的消息");
    let messages = [history, msg("m2", "正在生成")];

    const store = {
      get activeSession() {
        return { messages };
      },
      busy: true,
    };
    vi.doMock("../store", () => ({ useStore: () => store }));

    const { default: MessageList } = await import("./MessageList");
    const { rerender } = render(<MessageList />);

    expect(renderCount["已经写完的消息"]).toBe(1);
    expect(renderCount["正在生成"]).toBe(1);

    // 模拟打字机推进：只有活动消息拿到新的 blocks 数组，
    // 历史消息沿用原对象引用——这正是 store.appendBlocks 的实际行为
    messages = [history, msg("m2", "正在生成…")];
    rerender(<MessageList />);

    expect(
      renderCount["已经写完的消息"],
      "历史消息的 Markdown 不应被重新解析——memo 失效或 store 丢了引用稳定性",
    ).toBe(1);
    expect(renderCount["正在生成…"]).toBe(1);
  });

  /** 兜底：memo 不能把内容变化也拦掉，否则流式输出会停住。 */
  it("活动消息内容变化时仍会重渲染", async () => {
    const renderCount: Record<string, number> = {};
    vi.doMock("./Markdown", () => ({
      default: ({ content }: { content: string }) => {
        renderCount[content] = (renderCount[content] ?? 0) + 1;
        return <div data-testid="md">{content}</div>;
      },
    }));

    let messages = [msg("m1", "第一段")];
    const store = {
      get activeSession() {
        return { messages };
      },
      busy: true,
    };
    vi.doMock("../store", () => ({ useStore: () => store }));

    const { default: MessageList } = await import("./MessageList");
    const { rerender } = render(<MessageList />);
    messages = [msg("m1", "第一段第二句")];
    rerender(<MessageList />);

    expect(renderCount["第一段第二句"]).toBe(1);
  });
});