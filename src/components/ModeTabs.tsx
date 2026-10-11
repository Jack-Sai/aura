import { MODES, MODE_HINT, MODE_LABEL, type Mode } from "../lib/mode";

interface Props {
  mode: Mode;
  onChange: (m: Mode) => void;
  disabled?: boolean;
}

/**
 * 模式切换胶囊。
 *
 * 语义是「下一次发送采用哪个模式」，而非锁定当前会话——
 * 用户可以在同一个会话里先规划、再执行，中途改主意是合法的。
 */
export default function ModeTabs({ mode, onChange, disabled }: Props) {
  return (
    <div
      role="tablist"
      aria-label="会话模式"
      className="flex items-center gap-0.5 rounded-full border border-line bg-surface p-0.5"
    >
      {MODES.map((m) => {
        const active = m === mode;
        return (
          <button
            key={m}
            type="button"
            role="tab"
            aria-selected={active}
            title={MODE_HINT[m]}
            disabled={disabled}
            onClick={() => onChange(m)}
            className={[
              "rounded-full px-2.5 py-1 text-xs transition-colors",
              active
                ? "bg-brand text-white"
                : "text-subtle hover:bg-bubble hover:text-foreground",
              disabled ? "cursor-not-allowed opacity-50" : "",
            ].join(" ")}
          >
            {MODE_LABEL[m]}
          </button>
        );
      })}
    </div>
  );
}