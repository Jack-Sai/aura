import { useCallback, useEffect, useState } from "react";

const STORAGE_KEY = "aura-theme";

export type Theme = "light" | "dark";

function initialTheme(): Theme {
  const saved = localStorage.getItem(STORAGE_KEY);
  return saved === "dark" ? "dark" : "light";
}

export function useTheme() {
  const [theme, setTheme] = useState<Theme>(initialTheme);

  const update = useCallback((next: Theme) => {
    setTheme(next);
    localStorage.setItem(STORAGE_KEY, next);
  }, []);

  useEffect(() => {
    document.documentElement.classList.toggle("dark", theme === "dark");
  }, [theme]);

  return { theme, setTheme: update };
}
