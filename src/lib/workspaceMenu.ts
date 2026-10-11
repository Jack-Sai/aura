/**
 * 工作区右键菜单的菜单项推导。
 *
 * 抽成纯函数是因为它是这个 bug 的全部逻辑：原先这段判断内联在
 * App 的 onContextMenu 里，既无法测试，也是缺陷长期存在的原因——
 * 右键当前工作区时 items 为空，既不弹自定义菜单，
 * 又因为 preventDefault 提前执行而连系统菜单也没有。
 *
 * 与 Rust 侧 mode.rs / parser.rs 抽出纯函数的动机一致：
 * 副作用留在组件里，决策留下可测的部分。
 */
export type MenuItem = {
  label: string;
  danger?: boolean;
  onClick: () => void;
};

/**
 * 推导工作区右键菜单项。
 *
 * 当前工作区不可删除（store 侧有硬保护），因此没有可提供项时
 * 返回空数组——调用方据此放行原生菜单，而不是吞掉事件。
 */
export function workspaceMenuItems(
  path: string,
  currentWorkspace: string,
  onRemove: (path: string) => void,
): MenuItem[] {
  if (path === currentWorkspace) return [];
  return [{ label: "删除", danger: true, onClick: () => onRemove(path) }];
}