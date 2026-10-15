// Showing a tool window and moving the focus into it, and leaving the narrowed scope on Escape.

import { tick } from "svelte";
import { changeset } from "../../state/changeset.svelte";
import { headerFacts } from "./headerFacts.svelte";
import { shell } from "./shell.svelte";
import { TOOLS } from "./tools";

/** Shows the bottom tool `id` and moves the focus into its list once it is up. */
export async function focusBottom(id: string): Promise<void> {
  shell.showBottom(id);
  await tick();
  const tool = TOOLS.bottom.find((t) => t.id === id);
  const panel = tool && document.querySelector<HTMLElement>(`[aria-label="${tool.label}"]`);
  (panel?.querySelector<HTMLElement>("[data-list]") ?? panel)?.focus();
}

/** Shows the timeline and moves the focus into it. */
export const focusTimeline = () => focusBottom("timeline");

/** Moves the focus into the Problems list. */
export const focusProblems = () => focusBottom("problems");

/** Moves the focus into the Find list. */
export const focusFind = () => focusBottom("find");

/** Shows the Pipelines tool on the review screen and moves the focus into it. */
export async function focusPipelines(): Promise<void> {
  shell.show("review");
  await focusBottom("pipelines");
}

/** Escape, with the focus nowhere in particular or in the timeline or the chip, goes back to all changes. */
export function leaveScope(e: KeyboardEvent) {
  if (e.key !== "Escape" || e.defaultPrevented || e.metaKey || e.ctrlKey || e.altKey || e.shiftKey) return;
  if (!headerFacts.scope?.narrowed) return;
  const at = e.target instanceof Element ? e.target : undefined;
  if (at && at !== document.body && !at.closest("[data-escapes-scope]")) return;
  e.preventDefault();
  void changeset.showWhole();
}
