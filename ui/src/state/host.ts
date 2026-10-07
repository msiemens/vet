// The Tauri host's commands for choosing and remembering repositories. A plain browser
// (development only) has none: no dialog, and nothing is remembered.

import type { MenuItem } from "../lib/ui/menuItem";

const inTauri = "__TAURI_INTERNALS__" in window;

/** Whether native folder and file dialogs exist. */
export const hasNativeDialogs = inTauri;

async function invoke<T>(command: string, args?: Record<string, unknown>): Promise<T> {
  const { invoke } = await import("@tauri-apps/api/core");
  return invoke<T>(command, args);
}

/**
 * Shows the window once the app header can act as its title bar. "native" means the host
 * fell back to the platform's title bar; a plain browser has no window to show.
 */
export async function showWindow(): Promise<"custom" | "native" | "browser"> {
  return inTauri ? invoke<"custom" | "native">("activate_and_show") : "browser";
}

/** macOS has a native menu bar, drawn by the host from `setNativeMenu`; elsewhere the window shows the menu itself. */
export const isMac = navigator.userAgent.includes("Mac");
export const hasNativeMenu = inTauri && isMac;

/** Only the host's window can be closed from inside. */
export const canQuit = inTauri;

/** Replaces the native menu bar. The host reports a chosen item through `onNativeMenu`. */
export async function setNativeMenu(items: MenuItem[]): Promise<void> {
  await invoke("set_menu", { items });
}

/** Calls `listener` with the id of each item chosen in the native menu bar; resolves with the unsubscribe function. */
export async function onNativeMenu(listener: (id: string) => void): Promise<() => void> {
  const { listen } = await import("@tauri-apps/api/event");
  return listen<string>("vet-menu", (event) => listener(event.payload));
}

export async function quit(): Promise<void> {
  await invoke("quit");
}

/** The native folder dialog; `null` if cancelled. Rejects with a message for a folder the backend cannot open. */
export async function pickRepository(): Promise<string | null> {
  return inTauri ? invoke<string | null>("pick_repository") : null;
}

/** The native file dialog; `null` if cancelled. Rejects with a message for a file the backend cannot see. */
export async function pickFile(): Promise<string | null> {
  return inTauri ? invoke<string | null>("pick_file") : null;
}

/** Backend-side paths, most recent first. */
export async function recentRepositories(): Promise<string[]> {
  return inTauri ? invoke<string[]>("recent_repositories") : [];
}

export async function rememberRepository(path: string): Promise<void> {
  if (inTauri) await invoke("remember_repository", { path });
}

export async function forgetRepository(path: string): Promise<void> {
  if (inTauri) await invoke("forget_repository", { path });
}
