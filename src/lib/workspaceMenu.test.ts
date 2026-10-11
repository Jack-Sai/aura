import { describe, expect, it, vi } from "vitest";
import { workspaceMenuItems } from "./workspaceMenu";

describe("workspaceMenuItems", () => {
  it("非当前工作区提供删除项", () => {
    const items = workspaceMenuItems("E:/other", "E:/current", vi.fn());
    expect(items).toHaveLength(1);
    expect(items[0].label).toBe("删除");
    expect(items[0].danger).toBe(true);
  });

  it("点击删除回传被删的路径", () => {
    const onRemove = vi.fn();
    const items = workspaceMenuItems("E:/other", "E:/current", onRemove);
    items[0].onClick();
    expect(onRemove).toHaveBeenCalledWith("E:/other");
  });

  /**
   * 这是缺陷本身的断言：右键当前工作区时返回空数组，
   * 调用方据此 **不调 preventDefault**，从而放行系统原生菜单。
   *
   * 回归背景：侧边栏第一个工作区恒等于当前工作区
   * （store 在初始化与切换时都把它顶到首位），旧实现因此永远算不出
   * 菜单项，却又已提前 preventDefault——自定义菜单不显示，
   * 系统菜单也被吞掉，右键等于什么都不会发生。
   */
  it("当前工作区返回空数组以放行原生菜单", () => {
    expect(workspaceMenuItems("E:/current", "E:/current", vi.fn())).toEqual([]);
  });

  it("路径大小写或分隔符差异不应误判为同一工作区", () => {
    // store 已做规范化，这里只保证不因简单相等判断而漏删
    expect(
      workspaceMenuItems("E:\\other", "E:/current", vi.fn()),
    ).toHaveLength(1);
  });
});