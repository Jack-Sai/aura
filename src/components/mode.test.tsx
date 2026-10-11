import { render, screen, within } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { beforeEach, describe, expect, it, vi } from "vitest";
import ModeTabs from "./ModeTabs";
import TodoPanel from "./TodoPanel";
import { normalizeTodos, type Todo } from "../lib/todo";

// TodoPanel 把折叠偏好写进 localStorage，同一文件内的用例共享 jsdom，
// 不清理会让「折叠」用例污染后续用例
beforeEach(() => {
  localStorage.clear();
});

const todos = (raw: Array<Partial<Todo>>): Todo[] =>
  normalizeTodos(raw.map((t) => ({ id: "s1", title: "步骤", status: "pending", ...t })));

describe("ModeTabs", () => {
  it("渲染三个模式", () => {
    render(<ModeTabs mode="chat" onChange={() => {}} />);
    expect(screen.getByRole("tab", { name: "对话" })).toBeInTheDocument();
    expect(screen.getByRole("tab", { name: "规划" })).toBeInTheDocument();
    expect(screen.getByRole("tab", { name: "执行" })).toBeInTheDocument();
  });

  it("当前模式标记为选中", () => {
    render(<ModeTabs mode="plan" onChange={() => {}} />);
    expect(screen.getByRole("tab", { name: "规划" })).toHaveAttribute(
      "aria-selected",
      "true",
    );
    expect(screen.getByRole("tab", { name: "对话" })).toHaveAttribute(
      "aria-selected",
      "false",
    );
  });

  it("点击切换模式", async () => {
    const onChange = vi.fn();
    render(<ModeTabs mode="chat" onChange={onChange} />);
    await userEvent.click(screen.getByRole("tab", { name: "执行" }));
    expect(onChange).toHaveBeenCalledWith("build");
  });

  it("禁用时不响应点击", async () => {
    const onChange = vi.fn();
    render(<ModeTabs mode="chat" onChange={onChange} disabled />);
    await userEvent.click(screen.getByRole("tab", { name: "规划" }));
    expect(onChange).not.toHaveBeenCalled();
  });
});

describe("TodoPanel", () => {
  it("空清单不渲染", () => {
    const { container } = render(<TodoPanel todos={[]} />);
    expect(container).toBeEmptyDOMElement();
  });

  it("展示步骤与进度", () => {
    render(
      <TodoPanel
        todos={todos([
          { id: "s1", title: "建表" },
          { id: "s2", title: "写测试", status: "done" },
        ])}
      />,
    );
    expect(screen.getByText("建表")).toBeInTheDocument();
    expect(screen.getByText("写测试")).toBeInTheDocument();
    expect(screen.getByText("1/2")).toBeInTheDocument();
  });

  it("已完成的步骤加删除线", () => {
    render(
      <TodoPanel
        todos={todos([{ id: "s1", title: "已完成项", status: "done" }])}
      />,
    );
    expect(screen.getByText("已完成项")).toHaveClass("line-through");
  });

  it("按进行中优先排序", () => {
    render(
      <TodoPanel
        todos={todos([
          { id: "s1", title: "第一步", status: "done" },
          { id: "s2", title: "第二步" },
          { id: "s3", title: "第三步", status: "in_progress" },
        ])}
      />,
    );
    const items = screen.getAllByRole("listitem");
    expect(items[0]).toHaveTextContent("第三步");
    expect(items[2]).toHaveTextContent("第一步");
  });

  it("点击删除回调带上步骤 id", async () => {
    const onDelete = vi.fn();
    render(
      <TodoPanel
        todos={todos([{ id: "s7", title: "待删项" }])}
        onDeleteStep={onDelete}
      />,
    );
    await userEvent.click(screen.getByRole("button", { name: /删除步骤：待删项/ }));
    expect(onDelete).toHaveBeenCalledWith("s7");
  });

  it("全部完成时禁用开始执行", () => {
    render(
      <TodoPanel
        todos={todos([
          { id: "s1", title: "甲", status: "done" },
          { id: "s2", title: "乙", status: "done" },
        ])}
        onStartBuild={() => {}}
      />,
    );
    expect(screen.getByRole("button", { name: /已全部完成/ })).toBeDisabled();
  });

  it("未全部完成时可点击开始执行", async () => {
    const onStart = vi.fn();
    render(
      <TodoPanel
        todos={todos([
          { id: "s1", title: "甲", status: "done" },
          { id: "s2", title: "乙" },
        ])}
        onStartBuild={onStart}
      />,
    );
    const btn = screen.getByRole("button", { name: /开始执行/ });
    expect(btn).toBeEnabled();
    await userEvent.click(btn);
    expect(onStart).toHaveBeenCalled();
  });

  it("busy 时禁用开始执行", () => {
    render(
      <TodoPanel
        todos={todos([{ id: "s1", title: "甲" }])}
        onStartBuild={() => {}}
        busy
      />,
    );
    expect(screen.getByRole("button", { name: /开始执行/ })).toBeDisabled();
  });

  it("折叠后隐藏步骤列表但保留标题", async () => {
    render(<TodoPanel todos={todos([{ id: "s1", title: "建表" }])} />);
    expect(screen.getByText("建表")).toBeInTheDocument();
    await userEvent.click(screen.getByRole("button", { expanded: true }));
    expect(screen.queryByText("建表")).not.toBeInTheDocument();
    // 标题与进度仍在，用户知道面板还在
    expect(screen.getByText("任务清单")).toBeInTheDocument();
    expect(screen.getByText("0/1")).toBeInTheDocument();
  });

  it("无 onDeleteStep 时不渲染删除按钮", () => {
    render(<TodoPanel todos={todos([{ id: "s1", title: "甲" }])} />);
    expect(screen.queryByRole("button", { name: /删除步骤/ })).toBeNull();
  });

  it("渲染为有序列表", () => {
    render(<TodoPanel todos={todos([{ id: "s1", title: "甲" }])} />);
    const list = screen.getByRole("list");
    expect(within(list).getAllByRole("listitem")).toHaveLength(1);
  });
});