import { readStored, writeStored } from "../storage.ts";

export type Theme = "system" | "light" | "dark";

export const THEMES: { id: Theme; label: string }[] = [
  { id: "system", label: "System" },
  { id: "light", label: "Light" },
  { id: "dark", label: "Dark" },
];

const KEY = "vet.theme";

export function applyTheme(theme: Theme) {
  const root = document.documentElement;
  if (theme === "system") root.removeAttribute("data-theme");
  else root.setAttribute("data-theme", theme);
  writeStored(KEY, theme);
}

export function storedTheme(): Theme {
  const value = readStored(KEY);
  return value === "light" || value === "dark" ? value : "system";
}
