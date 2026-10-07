/** A tool rail's selected tool, and whether its sidebar is shown. */
export interface ToolWindow {
  tool: string;
  open: boolean;
}

/** The selected tool's window hides or shows; any other tool is selected and shown. */
export function toggleTool(current: ToolWindow, id: string): ToolWindow {
  return current.tool === id ? { tool: id, open: !current.open } : { tool: id, open: true };
}
