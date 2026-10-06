export type Theme = "system" | "light" | "dark";

const KEY = "vet.theme";

export function applyTheme(theme: Theme) {
  const root = document.documentElement;
  if (theme === "system") root.removeAttribute("data-theme");
  else root.setAttribute("data-theme", theme);
  try {
    localStorage.setItem(KEY, theme);
  } catch {
    // Storage may be unavailable; the theme then lasts for this session only.
  }
}

export function storedTheme(): Theme {
  try {
    const value = localStorage.getItem(KEY);
    if (value === "light" || value === "dark") return value;
  } catch {
    // Storage may be unavailable; fall back to the system theme.
  }
  return "system";
}
