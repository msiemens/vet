// How a severity is drawn, wherever it shows: its icon and its colour.
import type { Severity } from "../../generated/Severity.ts";
import { type IconName, iconSvg } from "./icons.ts";

/** A hint is drawn as info. */
export const severityIcon = (severity: Severity): IconName => (severity === "hint" ? "info" : severity);

export const severityColor = (severity: Severity): string => `var(--severity-${severityIcon(severity)})`;

/** The icon in its colour, for code that builds DOM outside Svelte (see `SeverityIcon.svelte`). */
export function severitySvg(severity: Severity, size = 14): string {
  return `<span class="severity-icon" style="display:inline-flex;flex:none;color:${severityColor(severity)}">${iconSvg(severityIcon(severity), size)}</span>`;
}
