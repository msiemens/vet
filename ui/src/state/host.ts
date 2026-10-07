// The Tauri host's commands: the window, the native menu and the file dialogs. A plain browser
// (development only) has none.

import type { MenuItem } from "../lib/ui/menuItem";

/** The window belongs to the Tauri host. */
export const inTauri = "__TAURI_INTERNALS__" in window;

/** Whether native folder and file dialogs exist. */
export const hasNativeDialogs = inTauri;

/** Calls a command of the Tauri host; only meaningful when `inTauri`. */
export async function invoke<T>(command: string, args?: Record<string, unknown>): Promise<T> {
  const { invoke } = await import("@tauri-apps/api/core");
  return invoke<T>(command, args);
}

/** The `#key=value&…` parameters a plain browser passes to the app in development. */
export const hashParams = (): URLSearchParams => new URLSearchParams(location.hash.slice(1));

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

export type StartupStep = "startingBackend" | "waitingForReady" | "loadingUi";

/** Where the host's startup is (the splash window shows it). A failure keeps the step it happened in. */
export interface StartupProgress {
  step: StartupStep;
  /** The WSL distro the backend runs in; none for a backend on this machine. */
  distro: string | null;
  error: string | null;
}

/** Calls `listener` with each startup step the host reports; resolves with the unsubscribe function. */
export async function onStartupProgress(listener: (progress: StartupProgress) => void): Promise<() => void> {
  const { listen } = await import("@tauri-apps/api/event");
  return listen<StartupProgress>("vet-startup", (event) => listener(event.payload));
}

/** Where startup is now, for a page that loaded after the host reported its first steps. */
export function startupProgress(): Promise<StartupProgress> {
  return invoke<StartupProgress>("startup_progress");
}

/** Starts the backend again after a failed start. */
export async function retryStartup(): Promise<void> {
  await invoke("retry_startup");
}

/** The app's version, as the host knows it. */
export async function appVersion(): Promise<string> {
  const { getVersion } = await import("@tauri-apps/api/app");
  return getVersion();
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
