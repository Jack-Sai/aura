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

// 这三项必须是模块级常量。在组件体内写字面量会每次 render 都造出新的
// 数组/对象，让 React.memo 的浅比较恒不相等——memo 形同虚设，
// 而 react-markdown 的解析（remark 词法分析 + 高亮 + KaTeX）并不便宜。
// 长对话里每个历史 block 都挂着一个 Markdown，全量重解析会拖垮流式渲染。
const REMARK_PLUGINS = [remarkGfm, remarkMath];
const REHYPE_PLUGINS = [rehypeKatex, rehypeHighlight];
const COMPONENTS = {
  a: ({ children, href }: { children?: React.ReactNode; href?: string }) => (
    <a href={href} target="_blank" rel="noopener noreferrer">
      {children}
    </a>
  ),
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