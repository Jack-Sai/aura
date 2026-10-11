import "@testing-library/jest-dom/vitest";
import { vi } from "vitest";

// Tauri 的 invoke/listen 在浏览器环境不存在。测试里一律替换为可控的桩，
// 让组件只依赖 api.ts 的薄封装，避免把 Tauri 细节泄漏进每个测试文件。
vi.mock("@tauri-apps/api/core", () => ({
  invoke: vi.fn(async () => null),
}));

vi.mock("@tauri-apps/api/event", () => ({
  listen: vi.fn(async () => () => {}),
}));