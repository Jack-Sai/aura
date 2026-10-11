import { useState } from "react";
import { ChevronRight, CircleAlert, Info } from "lucide-react";

export function ActionRow({
  text,
  ok,
  detail,
}: {
  text: string;
  ok?: boolean;
  detail?: string;
}) {
  // 失败默认展开，便于直接看到原因；成功默认折叠避免刷屏
  const [open, setOpen] = useState(ok === false);
  const failed = ok === false;
  const hasDetail = Boolean(detail && detail.trim());

  return (
    <div className="font-mono text-[13px] leading-relaxed">
      <div className="flex items-start gap-2">
        {hasDetail ? (
          <button
            type="button"
            onClick={() => setOpen((v) => !v)}
            aria-expanded={open}
            aria-label={open ? "收起输出" : "展开输出"}
            className="mt-1 shrink-0 text-subtle transition-transform hover:text-foreground"
          >
            <ChevronRight
              size={13}
              className={`transition-transform ${open ? "rotate-90" : ""}`}
            />
          </button>
        ) : failed ? (
          <CircleAlert size={13} className="mt-1 shrink-0 text-danger" />
        ) : (
          <ChevronRight size={13} className="mt-1 shrink-0" />
        )}
        <span
          className={`min-w-0 break-all ${failed ? "text-danger" : "text-subtle"}`}
        >
          {text}
        </span>
      </div>
      {hasDetail && open && (
        <pre className="scrollbar-hidden ml-5 mt-1.5 max-h-64 overflow-auto whitespace-pre-wrap break-words rounded-tag border border-line bg-surface-sunken px-2.5 py-2 text-[12px] leading-relaxed text-subtle">
          {detail}
        </pre>
      )}
    </div>
  );
}

export function NoticeRow({ text }: { text: string }) {
  return (
    <div className="flex items-start gap-2 font-mono text-[13px] leading-relaxed text-subtle">
      <Info size={13} className="mt-1 shrink-0" />
      <span className="min-w-0 break-all">{text}</span>
    </div>
  );
}

export function ErrorRow({ text }: { text: string }) {
  return (
    <div className="flex items-start gap-2 font-mono text-[13px] leading-relaxed text-danger">
      <CircleAlert size={13} className="mt-1 shrink-0" />
      <span className="min-w-0 break-all">{text}</span>
    </div>
  );
}