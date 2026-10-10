import { useCallback } from "react";
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

export default function Markdown({ content }: Props) {
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
        remarkPlugins={[remarkGfm, remarkMath]}
        rehypePlugins={[rehypeKatex, rehypeHighlight]}
        components={{
          a: ({ children, href }) => (
            <a href={href} target="_blank" rel="noopener noreferrer">
              {children}
            </a>
          ),
        }}
      >
        {content}
      </ReactMarkdown>
    </div>
  );
}