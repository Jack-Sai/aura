import { act, render, waitFor } from "@testing-library/react";
import { beforeEach, describe, expect, it, vi } from "vitest";

/**
 * 流式输出时 block 的显示顺序（集成层）。
 *
 * 纯函数级的 `appendText` 测试**抓不到**这个 bug——「末尾追加」恰好也能
 * 得到正确顺序，无法区分对错。真正的错在于：动作卡片走直接 append，
 * 不经过打字机缓冲；而模型「先说一句」的那句话此刻还躺在 buffer 里，
 * 于是动作卡片被排到了那句话**前面**。
 *
 * 只能把事件流驱动起来，从渲染结果上验证顺序。
 */
type Handler = (e: { payload: unknown }) => void;

const handlers = new Map<string, Handler>();

beforeEach(() => {
  handlers.clear();
  vi.resetModules();
  // send_message 永不 resolve，把这一轮「挂住」，才能在流进行中注入事件
  vi.doMock("@tauri-apps/api/core", () => ({
    invoke: vi.fn(async (cmd: string) => {
      switch (cmd) {
        case "get_workspace":
          return "E:/ws";
        case "load_sessions":
          return [];
        case "get_models":
          return [];
        case "get_selected_model":
          return "";
        case "send_message":
          return new Promise(() => {});
        default:
          return null;
      }
    }),
  }));
  vi.doMock("@tauri-apps/api/event", () => ({
    listen: vi.fn(async (name: string, cb: Handler) => {
      handlers.set(name, cb);
      return () => handlers.delete(name);
    }),
  }));
});

function fire(name: string, payload: unknown) {
  const h = handlers.get(name);
  if (!h) {
    throw new Error(
      `事件 ${name} 没有监听者，已注册：${[...handlers.keys()].join(", ")}`,
    );
  }
  act(() => h({ payload }));
}

async function mountAndSend(text: string) {
  const store = await import("../store");
  let api: ReturnType<typeof store.useStore> | null = null;

  function Probe() {
    api = store.useStore();
    const msgs = api.activeSession?.messages ?? [];
    return (
      <div>
        {msgs.map((m, i) =>
          m.role === "user" ? (
            <div key={i} data-kind="user" data-text={m.content} />
          ) : (
            <div key={i}>
              {m.blocks.map((b, j) => (
                <div key={j} data-kind={b.kind} data-text={b.text} />
              ))}
            </div>
          ),
        )}
      </div>
    );
  }

  const { StoreProvider } = store;
  render(
    <StoreProvider>
      <Probe />
    </StoreProvider>,
  );

  await waitFor(() => expect(api?.activeId).toBeTruthy());
  const sessionId = api!.activeId;
  act(() => {
    void api!.sendMessage(text);
  });
  return { sessionId };
}

function kinds(): string[] {
  return [...document.querySelectorAll("[data-kind]")].map((e) =>
    `${e.getAttribute("data-kind")}:${e.getAttribute("data-text")}`,
  );
}

describe("流式输出时 block 的显示顺序", () => {
  it("模型先说话再调工具时，动作卡片排在那句话之后", async () => {
    const { sessionId } = await mountAndSend("帮我看看");

    // 模型先输出一句思考，还没被打字机消费（打字机 30ms 一跳）
    fire("agent:chunk", { session_id: sessionId, text: "先说一句" });
    // 紧接着调用工具
    fire("agent:action", { session_id: sessionId, text: "Read a.rs", ok: true });

    expect(kinds()).toEqual([
      "user:帮我看看",
      "text:先说一句",
      "action:Read a.rs",
    ]);
  });

  it("多次交替说话与调工具，顺序始终与输出一致", async () => {
    const { sessionId } = await mountAndSend("干活");

    fire("agent:chunk", { session_id: sessionId, text: "开始" });
    fire("agent:action", { session_id: sessionId, text: "A", ok: true });
    fire("agent:chunk", { session_id: sessionId, text: "接着" });
    fire("agent:action", { session_id: sessionId, text: "B", ok: true });
    fire("agent:chunk", { session_id: sessionId, text: "收尾" });
    // 收尾的正文此刻还在缓冲区（打字机 30ms 一跳），
    // 由 done 事件触发排空——否则断言时它还没落进 blocks
    fire("agent:done", { session_id: sessionId });

    expect(kinds()).toEqual([
      "user:干活",
      "text:开始",
      "action:A",
      "text:接着",
      "action:B",
      "text:收尾",
    ]);
  });

  it("提示块（notice）同样排在其之前的正文之后", async () => {
    const { sessionId } = await mountAndSend("测一下");

    fire("agent:chunk", { session_id: sessionId, text: "前置说明" });
    fire("agent:notice", { session_id: sessionId, text: "已自动降级至 X" });

    expect(kinds()).toEqual([
      "user:测一下",
      "text:前置说明",
      "notice:已自动降级至 X",
    ]);
  });

  it("没有前置正文时，动作卡片排在最前", async () => {
    const { sessionId } = await mountAndSend("直接干活");
    fire("agent:action", { session_id: sessionId, text: "Write b.rs", ok: true });
    expect(kinds()).toEqual(["user:直接干活", "action:Write b.rs"]);
  });
});