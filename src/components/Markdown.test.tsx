import { render, screen } from "@testing-library/react";
import { beforeEach, describe, expect, it, vi } from "vitest";

/**
 * 代码块语言标签。
 *
 * 模型几乎总会带上 ```rust 这类语言标识，但界面上看不到——用户只能靠
 * 猜。这里钉住「有标识就显示，没有就不显示」。
 */
beforeEach(() => {
  vi.resetModules();
  vi.doUnmock("../store");
});

async function renderMd(content: string) {
  const { default: Markdown } = await import("./Markdown");
  return render(<Markdown content={content} />);
}

describe("Markdown 代码块语言标签", () => {
  it("有语言标识时显示标签", async () => {
    await renderMd("```rust\nfn main() {}\n```");
    expect(screen.getByText("Rust")).toBeInTheDocument();
  });

  it("语言标识大小写不敏感", async () => {
    await renderMd("```TypeScript\nconst a = 1;\n```");
    expect(screen.getByText("TypeScript")).toBeInTheDocument();
  });

  it("未知语言直接回退到标识本身", async () => {
    // 不该把 lang-xx 猜成一个错误的名字，宁可原样显示
    await renderMd("```nim\ndiscard\n```");
    expect(screen.getByText("nim")).toBeInTheDocument();
  });

  it("没有语言标识时不显示标签", async () => {
    await renderMd("```\nplain text\n```");
    expect(screen.queryByText(/^\w+$/, { selector: ".md-code-lang" })).toBeNull();
  });

  it("行内代码不会被误判成代码块", async () => {
    await renderMd("这是 `const x = 1;` 这样的行内代码。");
    expect(screen.queryByText("TypeScript")).toBeNull();
    expect(screen.queryByText("const")).toBeNull();
  });
});