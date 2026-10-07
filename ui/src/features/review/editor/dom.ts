// Plain DOM builders for the editor's own widgets (headers, hunk bars, cards), which are not Svelte.

import { type IconName, iconSvg } from "../../../lib/ui/icons.ts";

export function el<K extends keyof HTMLElementTagNameMap>(
  tag: K,
  className: string,
  text?: string,
): HTMLElementTagNameMap[K] {
  const e = document.createElement(tag);
  e.className = className;
  if (text !== undefined) e.textContent = text;
  return e;
}

/** A flat button with an icon; `className` also sets its size (`vet-icon-btn` is the 22px one). */
export function iconButton(
  label: string,
  icon: IconName,
  onclick: () => void,
  className = "vet-icon-btn",
  size = 13,
  stroke = 1.8,
): HTMLButtonElement {
  const button = el("button", `flat ${className}`);
  button.type = "button";
  button.ariaLabel = button.title = label;
  button.innerHTML = iconSvg(icon, size, stroke);
  button.addEventListener("click", onclick);
  return button;
}
