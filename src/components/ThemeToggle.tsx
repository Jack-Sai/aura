import { Moon, Sun } from "lucide-react";
import type { Theme } from "../hooks/useTheme";

interface Props {
  theme: Theme;
  onToggle: () => void;
}

export default function ThemeToggle({ theme, onToggle }: Props) {
  return (
    <button
      type="button"
      onClick={onToggle}
      aria-label={theme === "light" ? "切换到暗色" : "切换到亮色"}
      className="rounded-full border border-line p-2 text-subtle transition-colors hover:bg-bubble hover:text-foreground"
    >
      {theme === "light" ? <Moon size={15} /> : <Sun size={15} />}
    </button>
  );
}
