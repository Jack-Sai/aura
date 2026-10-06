import { ChevronRight, CircleAlert, Info } from "lucide-react";

export function ActionRow({ text }: { text: string }) {
  return (
    <div className="flex items-start gap-2 font-mono text-[13px] leading-relaxed text-subtle">
      <ChevronRight size={13} className="mt-1 shrink-0" />
      <span className="min-w-0 break-all">{text}</span>
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
