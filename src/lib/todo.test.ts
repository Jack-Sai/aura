import { describe, expect, it } from "vitest";
import {
  applyTodoUpdate,
  displayOrder,
  doneCount,
  isTodoStatus,
  nextTodoId,
  normalizeTodos,
} from "./todo";

describe("normalizeTodos", () => {
  it("接受合法数据", () => {
    const todos = normalizeTodos([
      { id: "s1", title: "建表", status: "pending" },
      { id: "s2", title: "写测试", status: "done" },
    ]);
    expect(todos).toHaveLength(2);
    expect(todos[1].status).toBe("done");
  });

  it("缺 status 时回落 pending", () => {
    // 绝不能把未知状态透传出去，否则渲染会落到「未知状态」分支
    const todos = normalizeTodos([{ id: "s1", title: "甲" }]);
    expect(todos[0].status).toBe("pending");
  });

  it("非法状态值回落 pending", () => {
    const todos = normalizeTodos([{ id: "s1", title: "甲", status: "inprogress" }]);
    expect(todos[0].status).toBe("pending");
  });

  it("丢弃空标题与空 id", () => {
    const todos = normalizeTodos([
      { id: "s1", title: "保留" },
      { id: "s2", title: "   " },
      { id: "", title: "无 id" },
    ]);
    expect(todos).toHaveLength(1);
    expect(todos[0].id).toBe("s1");
  });

  it("去重 id", () => {
    const todos = normalizeTodos([
      { id: "s1", title: "先来" },
      { id: "s1", title: "重复" },
    ]);
    expect(todos).toHaveLength(1);
    expect(todos[0].title).toBe("先来");
  });

  it("非数组输入返回空清单", () => {
    expect(normalizeTodos(null)).toEqual([]);
    expect(normalizeTodos(undefined)).toEqual([]);
    expect(normalizeTodos("nope")).toEqual([]);
    expect(normalizeTodos([null, 1, "x"])).toEqual([]);
  });

  it("去除首尾空白", () => {
    const todos = normalizeTodos([{ id: " s1 ", title: "  甲  " }]);
    expect(todos[0].id).toBe("s1");
    expect(todos[0].title).toBe("甲");
  });
});

describe("isTodoStatus", () => {
  it("只认后端约定的三个值", () => {
    expect(isTodoStatus("pending")).toBe(true);
    expect(isTodoStatus("in_progress")).toBe(true);
    expect(isTodoStatus("done")).toBe(true);
    expect(isTodoStatus("inprogress")).toBe(false);
    expect(isTodoStatus("completed")).toBe(false);
  });
});

describe("doneCount", () => {
  it("只数已完成", () => {
    const todos = normalizeTodos([
      { id: "s1", title: "a", status: "done" },
      { id: "s2", title: "b", status: "in_progress" },
      { id: "s3", title: "c", status: "done" },
    ]);
    expect(doneCount(todos)).toBe(2);
  });

  it("空清单为 0", () => {
    expect(doneCount([])).toBe(0);
  });
});

describe("displayOrder", () => {
  it("进行中在前、已完成沉底", () => {
    const todos = normalizeTodos([
      { id: "s1", title: "a", status: "done" },
      { id: "s2", title: "b", status: "pending" },
      { id: "s3", title: "c", status: "in_progress" },
    ]);
    expect(displayOrder(todos).map((t) => t.id)).toEqual(["s3", "s2", "s1"]);
  });

  it("同组内保持原顺序", () => {
    const todos = normalizeTodos([
      { id: "s1", title: "a" },
      { id: "s2", title: "b" },
      { id: "s3", title: "c", status: "done" },
      { id: "s4", title: "d", status: "done" },
    ]);
    expect(displayOrder(todos).map((t) => t.id)).toEqual(["s1", "s2", "s3", "s4"]);
  });

  it("不修改入参数组", () => {
    const todos = normalizeTodos([
      { id: "s1", title: "a", status: "done" },
      { id: "s2", title: "b" },
    ]);
    displayOrder(todos);
    expect(todos[0].id).toBe("s1");
  });
});

describe("applyTodoUpdate", () => {
  it("命中即更新并返回 true", () => {
    const todos = normalizeTodos([{ id: "s1", title: "a" }]);
    expect(applyTodoUpdate(todos, "s1", "done")).toBe(true);
    expect(todos[0].status).toBe("done");
  });

  it("未命中返回 false 且不改数据", () => {
    const todos = normalizeTodos([{ id: "s1", title: "a" }]);
    expect(applyTodoUpdate(todos, "nope", "done")).toBe(false);
    expect(todos[0].status).toBe("pending");
  });
});

describe("nextTodoId", () => {
  it("按长度+1 递增", () => {
    const todos = normalizeTodos([
      { id: "s1", title: "a" },
      { id: "s2", title: "b" },
    ]);
    expect(nextTodoId(todos)).toBe("s3");
  });

  it("跳过已被占用的编号", () => {
    // 用户删掉中间几步后，长度+1 可能正好撞上已有 id
    const todos = normalizeTodos([
      { id: "s1", title: "a" },
      { id: "s3", title: "b" },
    ]);
    const next = nextTodoId(todos);
    expect(todos.map((t) => t.id)).not.toContain(next);
    expect(next).toBe("s4");
  });

  it("连续占用时会一路递增直到空缺", () => {
    const todos = normalizeTodos([
      { id: "s1", title: "a" },
      { id: "s2", title: "b" },
      { id: "s3", title: "c" },
      { id: "s4", title: "d" },
    ]);
    expect(nextTodoId(todos)).toBe("s5");
  });

  it("空清单从 s1 开始", () => {
    expect(nextTodoId([])).toBe("s1");
  });
});