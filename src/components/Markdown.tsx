import { memo, useCallback } from "react";
import ReactMarkdown from "react-markdown";
import rehypeHighlight from "rehype-highlight";
import rehypeKatex from "rehype-katex";
import remarkGfm from "remark-gfm";
import remarkMath from "remark-math";
import { openUrl } from "@tauri-apps/plugin-opener";
import "katex/dist/katex.min.css";

interface Props {
  content: string;
}

/** 绝对 URL（含 http/https）才拦截外跳；相对链接与锚点交给默认行为 */
function isExternal(href: string): boolean {
  return /^https?:\/\//i.test(href);
}

/**
 * 语言标签显示名。模型输出的语言标识种类很多，只映射常见的，
 * 其余直接回退到标识本身（`tsx`、`sh` 这类反而比猜一个中文名更准确）。
 */
const LANG_LABELS: Record<string, string> = {
  bash: "Bash",
  c: "C",
  cpp: "C++",
  cs: "C#",
  css: "CSS",
  diff: "Diff",
  go: "Go",
  graphql: "GraphQL",
  html: "HTML",
  java: "Java",
  javascript: "JavaScript",
  json: "JSON",
  jsx: "JSX",
  kotlin: "Kotlin",
  lua: "Lua",
  markdown: "Markdown",
  php: "PHP",
  python: "Python",
  py: "Python",
  rb: "Ruby",
  rs: "Rust",
  rust: "Rust",
  scss: "SCSS",
  sh: "Shell",
  shell: "Shell",
  sql: "SQL",
  swift: "Swift",
  toml: "TOML",
  ts: "TypeScript",
  tsx: "TSX",
  typescript: "TypeScript",
  vue: "Vue",
  xml: "XML",
  yaml: "YAML",
  yml: "YAML",
  zig: "Zig",
};

function langLabel(raw: string): string {
  const key = raw.toLowerCase();
  return LANG_LABELS[key] ?? raw;
}

// 这三项必须是模块级常量。在组件体内写字面量会每次 render 都造出新的
// 数组/对象，让 React.memo 的浅比较恒不相等——memo 形同虚设，
// 而 react-markdown 的解析（remark 词法分析 + 高亮 + KaTeX）并不便宜。
// 长对话里每个历史 block 都挂着一个 Markdown，全量重解析会拖垮流式渲染。
const REMARK_PLUGINS = [remarkGfm, remarkMath];
const REHYPE_PLUGINS = [rehypeKatex, rehypeHighlight];

/**
 * react-markdown 没有 `isBlock` 参数，只能靠 children 结构判断：
 * 代码块是 `<pre><code>…</code></pre>`，行内代码是裸 `<code>`。
 */
function pickLang(children: React.ReactNode): string | null {
  const child = Array.isArray(children) ? children[0] : children;
  if (!child || typeof child !== "object" || !("props" in child)) return null;
  const cls = (child as { props?: { className?: string } }).props?.className;
  const m = /language-([\w+#-]+)/.exec(cls ?? "");
  return m ? m[1] : null;
}

const COMPONENTS = {
  a: ({ children, href }: { children?: React.ReactNode; href?: string }) => (
    <a href={href} target="_blank" rel="noopener noreferrer">
      {children}
    </a>
  ),
  /** 代码块右上角标出语言；行内代码由 CSS 自行处理，不在此处介入 */
  pre: ({ children }: { children?: React.ReactNode }) => {
    const lang = pickLang(children);
    return (
      <div className="md-code">
        {lang && <span className="md-code-lang">{langLabel(lang)}</span>}
        <pre>{children}</pre>
      </div>
    );
  },
};

const Markdown = memo(function Markdown({ content }: Props) {
  /**
   * 模型输出里的链接一律用系统浏览器打开，绝不让 WebView 跟随导航。
   *
   * 否则 WebView 会跳到该页面，把整个 React 应用替换掉；而窗口是
   * `decorations: false`，连标题栏的关闭按钮都不存在，用户只能杀进程。
   */
  const handleClick = useCallback((e: React.MouseEvent<HTMLDivElement>) => {
    const anchor = (e.target as HTMLElement).closest("a");
    if (!anchor) return;
    const href = anchor.getAttribute("href") ?? "";
    if (!isExternal(href)) return;
    e.preventDefault();
    void openUrl(href).catch(() => {
      /* 打开失败时静默，避免打断对话 */
    });
  }, []);

  return (
    <div className="md" onClick={handleClick}>
      <ReactMarkdown
        remarkPlugins={REMARK_PLUGINS}
        rehypePlugins={REHYPE_PLUGINS}
        components={COMPONENTS}
      >
        {content}
      </ReactMarkdown>
    </div>
  );
});

export default Markdown;