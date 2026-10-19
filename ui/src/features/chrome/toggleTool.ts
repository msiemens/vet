/** A tool rail's selected tool, and whether its sidebar is shown. */
export interface ToolWindow {
  tool: string;
  open: boolean;
}

/** The window kept as `json`, if it names one of `tools` and says whether it is open. */
export function parseToolWindow(json: unknown, tools: readonly string[]): ToolWindow | undefined {
  if (typeof json !== "object" || json === null) return undefined;
  const { tool, open } = json as Record<string, unknown>;
  if (typeof open !== "boolean" || typeof tool !== "string" || !tools.includes(tool)) return undefined;
  return { tool, open };
}

/** The selected tool's window hides or shows; any other tool is selected and shown. */
export function toggleTool(current: ToolWindow, id: string): ToolWindow {
  return current.tool === id ? { tool: id, open: !current.open } : { tool: id, open: true };
}

/** The bottom panel shown after `id` is toggled: one at a time, so the tool shown gives way to another, and hides itself. */
export function toggleBottom(shown: string | undefined, id: string): string | undefined {
  return shown === id ? undefined : id;
}
