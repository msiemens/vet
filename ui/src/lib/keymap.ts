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

/** A key binding as `matches` reads it, with the text that shows it to the viewer. */
export interface Binding {
  spec: string;
  label: string;
}

/** The modifier `Mod` stands for in a spec: Cmd on macOS, Ctrl elsewhere. */
export type ModKey = "Ctrl" | "Meta";

/** A spec with `Mod` replaced by the platform's modifier, as `matches` reads it. */
export function bindingOf(spec: string, mod: ModKey): Binding {
  const parts = spec.split("-").map((part) => (part === "Mod" ? mod : part));
  return {
    spec: parts.join("-"),
    label: parts
      .map((part) => (part.length === 1 ? part.toUpperCase() : part === "Meta" ? "Cmd" : part.replace(/^Arrow/, "")))
      .join("+"),
  };
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

const MODIFIER_ORDER = ["Ctrl", "Alt", "Shift", "Meta"];

/** `spec` with its modifiers in a fixed order and a letter in lower case, so equal bindings compare equal. */
export function canonicalSpec(spec: string): string {
  const parts = spec.split("-");
  const name = parts.at(-1) ?? "";
  const mods = new Set(parts.slice(0, -1));
  return [...MODIFIER_ORDER.filter((m) => mods.has(m)), name.length === 1 ? name.toLowerCase() : name].join("-");
}

/** Keys that only change what the next key means. */
const MODIFIER_KEYS = new Set(["Control", "Alt", "AltGraph", "Shift", "Meta", "OS", "CapsLock", "NumLock", "Fn"]);

/**
 * The spec that `matches` reads as this event: the way a viewer records a binding by pressing it.
 * `null` for what cannot be one: a lone modifier, text input (so the AltGr, IME and dead-key
 * safety of `matches` holds for recorded bindings too) and a key `matches` cannot tell apart
 * (punctuation).
 */
export function specOf(e: KeyLike): string | null {
  if (isTextInput(e) || MODIFIER_KEYS.has(e.key)) return null;
  const char = baseChar(e.code);
  let name: string;
  if (char !== null) name = char;
  else if (e.key === " ") name = "Space";
  else if (e.key.length > 1 && e.key !== "Dead" && e.key !== "Process" && e.key !== "Unidentified") name = e.key;
  else return null;
  const mods = [e.ctrlKey && "Ctrl", e.altKey && "Alt", e.shiftKey && "Shift", e.metaKey && "Meta"];
  return [...mods.filter((m) => m !== false), name].join("-");
}

/**
 * Matches `spec` like `Ctrl-Alt-ArrowDown`, `Alt-s`, `F3`, `Space` or `[`. Letters and digits
 * match the physical key (`code`), so layouts do not matter; named keys and punctuation
 * match `key`, the character the layout types. Modifiers must match exactly. Text input never matches.
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
  if (name.length === 1 && /[a-z0-9]/i.test(name)) return baseChar(e.code) === name.toLowerCase();
  return e.key === (name === "Space" ? " " : name);
}
