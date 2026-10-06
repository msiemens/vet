// Keybindings that never fire on text input (SPEC §12.0 input correctness).
//
// On Windows, AltGr is reported as Ctrl+Alt, so `Ctrl-Alt-Q` would also match
// AltGr+Q (`@` on a German layout). On macOS, Option+key types characters. Dead
// keys and IME composition arrive as `Dead` / `Process` / keyCode 229. All of
// these are text input: no binding may consume them.

export interface KeyLike {
  key: string;
  code: string;
  ctrlKey: boolean;
  altKey: boolean;
  shiftKey: boolean;
  metaKey: boolean;
  isComposing?: boolean;
  keyCode?: number;
  getModifierState?(modifier: string): boolean;
}

/** The character a letter or digit key types without modifiers, if `code` is one. */
function baseChar(code: string): string | null {
  if (/^Key[A-Z]$/.test(code)) return code.slice(3).toLowerCase();
  if (/^Digit[0-9]$/.test(code)) return code.slice(5);
  return null;
}

/** True when the event types (or is part of typing) a character. */
export function isTextInput(e: KeyLike): boolean {
  if (e.isComposing || e.keyCode === 229 || e.key === "Dead" || e.key === "Process") return true;
  if (e.getModifierState?.("AltGraph")) return true;
  // Alt (or Ctrl+Alt on Windows) produced a character other than the key's own
  // letter: AltGr on Windows/Linux, Option on macOS.
  return e.altKey && e.key.length === 1 && e.key.toLowerCase() !== baseChar(e.code);
}

/**
 * Matches `spec` like `Ctrl-Alt-ArrowDown`, `Alt-s`, `F3` or `Space`. Letters and digits
 * match the physical key (`code`), so layouts do not matter; named keys match
 * `key`. Modifiers must match exactly. Text input never matches.
 */
export function matches(spec: string, e: KeyLike): boolean {
  if (isTextInput(e)) return false;
  const parts = spec.split("-");
  const name = parts.at(-1) ?? "";
  const mods = new Set(parts.slice(0, -1));
  if (
    mods.has("Ctrl") !== e.ctrlKey ||
    mods.has("Alt") !== e.altKey ||
    mods.has("Shift") !== e.shiftKey ||
    mods.has("Meta") !== e.metaKey
  )
    return false;
  if (name.length === 1) return baseChar(e.code) === name.toLowerCase();
  return e.key === (name === "Space" ? " " : name);
}
