import { render, screen } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { beforeEach, describe, expect, it, vi } from "vitest";
import { normalizeTodos } from "../lib/todo";

beforeEach(() => {
  localStorage.clear();
  vi.resetModules();
  vi.doUnmock("../store");
});

/**
 * RightPanel 依赖 store，这里 mock 掉 useStore 而不是渲染真实 Provider。
 * 真实 Provider 会拉起 Tauri 的 invoke/listen，在单测里没有意义。
 */
async function mount({ todos, mode }: { todos: unknown[]; mode: string }) {
  const store = {
    sessions: [
      {
        id: "s1",
        title: "t",
        workspace: "/",
        messages: [],
        todos: normalizeTodos(todos),
      },
    ],
    activeId: "s1",
    mode,
    deleteTodoStep: vi.fn(),
    addTodoStep: vi.fn(),
    setTodoStatus: vi.fn(),
  };
  vi.doMock("../store", () => ({ useStore: () => store }));
  const { default: RightPanel } = await import("./RightPanel");
  return render(<RightPanel />);
}

describe("RightPanel", () => {
  it("有清单且处于执行模式时显示", async () => {
    await mount({ todos: [{ id: "s1", title: "建表" }], mode: "build" });
    expect(screen.getByText("建表")).toBeInTheDocument();
  });

  it("规划模式下同样显示", async () => {
    await mount({ todos: [{ id: "s1", title: "调研" }], mode: "plan" });
    expect(screen.getByText("调研")).toBeInTheDocument();
  });

  /// 对话模式是纯问答，不涉及流程，不应占用横向空间
  it("对话模式下不显示", async () => {
    await mount({ todos: [{ id: "s1", title: "不应出现" }], mode: "chat" });
    expect(screen.queryByText("不应出现")).toBeNull();
  });

  it("无清单时整体不渲染，不留空面板", async () => {
    await mount({ todos: [], mode: "build" });
    expect(screen.queryByText(/任务面板/)).toBeNull();
    expect(screen.queryByRole("complementary")).toBeNull();
  });

  it("可收起为窄条，收起后仍能看到步骤数", async () => {
    await mount({
      todos: [
        { id: "s1", title: "建表" },
        { id: "s2", title: "写测试" },
      ],
      mode: "build",
    });
    await userEvent.click(screen.getByRole("button", { name: "收起任务面板" }));
    expect(screen.queryByText("建表")).toBeNull();
    expect(screen.getByRole("button", { name: "展开任务面板" })).toBeInTheDocument();
    expect(screen.getByText(/任务 2/)).toBeInTheDocument();
  });

  it("收起态可再次展开", async () => {
    await mount({ todos: [{ id: "s1", title: "建表" }], mode: "build" });
    await userEvent.click(screen.getByRole("button", { name: "收起任务面板" }));
    await userEvent.click(screen.getByRole("button", { name: "展开任务面板" }));
    expect(screen.getByText("建表")).toBeInTheDocument();
  });

  it("折叠偏好持久化到 localStorage", async () => {
    await mount({ todos: [{ id: "s1", title: "建表" }], mode: "build" });
    await userEvent.click(screen.getByRole("button", { name: "收起任务面板" }));
    expect(localStorage.getItem("aura-right-panel-collapsed")).toBe("1");
  });
});