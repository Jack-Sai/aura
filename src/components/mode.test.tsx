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
    expect(screen.getByRole("tab", { name: "Chat" })).toBeInTheDocument();
    expect(screen.getByRole("tab", { name: "Plan" })).toBeInTheDocument();
    expect(screen.getByRole("tab", { name: "Build" })).toBeInTheDocument();
  });

  it("当前模式标记为选中", () => {
    render(<ModeTabs mode="plan" onChange={() => {}} />);
    expect(screen.getByRole("tab", { name: "Plan" })).toHaveAttribute(
      "aria-selected",
      "true",
    );
    expect(screen.getByRole("tab", { name: "Chat" })).toHaveAttribute(
      "aria-selected",
      "false",
    );
  });

  it("点击切换模式", async () => {
    const onChange = vi.fn();
    render(<ModeTabs mode="chat" onChange={onChange} />);
    await userEvent.click(screen.getByRole("tab", { name: "Build" }));
    expect(onChange).toHaveBeenCalledWith("build");
  });

  it("禁用时不响应点击", async () => {
    const onChange = vi.fn();
    render(<ModeTabs mode="chat" onChange={onChange} disabled />);
    await userEvent.click(screen.getByRole("tab", { name: "Plan" }));
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

  /// 模式只能由用户手动切换，面板不得提供任何自动流转入口。
  ///
  /// 自动切模式会剥夺用户表达意图的机会——想补充信息、想重新规划时
  /// 都没有正常路径。
  it("不提供任何自动流转入口", () => {
    render(
      <TodoPanel
        todos={todos([
          { id: "s1", title: "甲", status: "done" },
          { id: "s2", title: "乙" },
        ])}
      />,
    );
    expect(screen.queryByRole("button", { name: /开始执行/ })).toBeNull();
    expect(screen.queryByRole("button", { name: /已全部完成/ })).toBeNull();
    expect(screen.queryByText(/切换到执行/)).toBeNull();
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

  describe("手动推进状态", () => {
    it("待执行 → 进行中", async () => {
      const onToggle = vi.fn();
      render(
        <TodoPanel
          todos={todos([{ id: "s1", title: "甲", status: "pending" }])}
          onToggleStep={onToggle}
        />,
      );
      await userEvent.click(screen.getByRole("button", { name: "步骤状态：待执行" }));
      expect(onToggle).toHaveBeenCalledWith("s1", "in_progress");
    });

    it("进行中 → 已完成", async () => {
      const onToggle = vi.fn();
      render(
        <TodoPanel
          todos={todos([{ id: "s1", title: "甲", status: "in_progress" }])}
          onToggleStep={onToggle}
        />,
      );
      await userEvent.click(screen.getByRole("button", { name: "步骤状态：进行中" }));
      expect(onToggle).toHaveBeenCalledWith("s1", "done");
    });

    it("已完成 → 待执行，可回退重做", async () => {
      const onToggle = vi.fn();
      render(
        <TodoPanel
          todos={todos([{ id: "s1", title: "甲", status: "done" }])}
          onToggleStep={onToggle}
        />,
      );
      await userEvent.click(screen.getByRole("button", { name: "步骤状态：已完成" }));
      expect(onToggle).toHaveBeenCalledWith("s1", "pending");
    });

    it("无 onToggleStep 时状态图标不可点击", () => {
      render(<TodoPanel todos={todos([{ id: "s1", title: "甲" }])} />);
      expect(screen.getByRole("button", { name: "步骤状态：待执行" })).toBeDisabled();
    });
  });

  describe("手动新增步骤", () => {
    it("输入后回车新增", async () => {
      const onAdd = vi.fn();
      render(
        <TodoPanel todos={todos([{ id: "s1", title: "甲" }])} onAddStep={onAdd} />,
      );
      const input = screen.getByRole("textbox", { name: "添加步骤" });
      await userEvent.type(input, "补充测试");
      await userEvent.keyboard("{Enter}");
      expect(onAdd).toHaveBeenCalledWith("补充测试");
    });

    it("空白内容不触发新增", async () => {
      const onAdd = vi.fn();
      render(
        <TodoPanel todos={todos([{ id: "s1", title: "甲" }])} onAddStep={onAdd} />,
      );
      await userEvent.type(screen.getByRole("textbox", { name: "添加步骤" }), "   ");
      await userEvent.keyboard("{Enter}");
      expect(onAdd).not.toHaveBeenCalled();
    });

    it("新增后清空输入框", async () => {
      render(<TodoPanel todos={todos([{ id: "s1", title: "甲" }])} onAddStep={() => {}} />);
      const input = screen.getByRole("textbox", { name: "添加步骤" }) as HTMLInputElement;
      await userEvent.type(input, "新步骤");
      await userEvent.keyboard("{Enter}");
      expect(input.value).toBe("");
    });

    it("无 onAddStep 时不渲染输入框", () => {
      render(<TodoPanel todos={todos([{ id: "s1", title: "甲" }])} />);
      expect(screen.queryByRole("textbox", { name: "添加步骤" })).toBeNull();
    });
  });
});