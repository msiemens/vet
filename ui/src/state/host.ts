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

/**
 * Gives the window the position and size of the welcome screen or of a project. Resolves once the
 * window has them, so the page changes what it shows after, not before.
 */
export async function setWindowMode(mode: "welcome" | "project"): Promise<void> {
  if (inTauri) await invoke("set_window_mode", { mode });
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

/** The host pops up native context menus; a plain browser draws them in the page. */
export const hasNativeContextMenu = inTauri;

/**
 * Pops up a context menu of the host at the viewport position `x`, `y` and resolves with the id of
 * the item chosen, or `null` when it closed without one.
 */
export async function popUpNativeMenu(items: MenuItem[], x: number, y: number): Promise<string | null> {
  return invoke<string | null>("show_context_menu", { items, x, y });
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

/** Only the host can start the backend again. */
export const canRestartBackend = inTauri;

/** Ends the backend and starts another; resolves once the new one listens. */
export async function restartBackend(): Promise<void> {
  await invoke("restart_backend");
}

/** Where the backend runs, as a project list names it: WSL, or this machine's system; none in a plain browser. */
export async function hostLabel(): Promise<string | undefined> {
  if (!inTauri) return undefined;
  if ((await startupProgress()).distro !== null) return "WSL";
  if (isMac) return "macOS";
  return navigator.userAgent.includes("Windows") ? "Windows" : "Linux";
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

/** Whether the personal access tokens survive the window: the host keeps them in the OS keychain. A plain browser has none. */
export const hasKeychain = inTauri;

/** The token the keychain holds for a host. */
export interface StoredCredential {
  host: string;
  token: string;
}

/** Every token in the keychain, for the hosts that have one. */
export async function storedCredentials(): Promise<StoredCredential[]> {
  return inTauri ? invoke<StoredCredential[]>("stored_credentials") : [];
}

/** The token the keychain holds for `host`, if any. */
export async function storedCredential(host: string): Promise<string | null> {
  return inTauri ? invoke<string | null>("stored_credential", { host }) : null;
}

/** Keeps `token` in the keychain under `host`. Rejects with a message if the keychain refuses. */
export async function storeCredential(host: string, token: string): Promise<void> {
  if (inTauri) await invoke("store_credential", { host, token });
}

export async function removeStoredCredential(host: string): Promise<void> {
  if (inTauri) await invoke("remove_credential", { host });
}

/** Shows a web address in the user's browser. A plain browser opens a tab. Rejects with a message if it cannot. */
export async function openUrl(url: string): Promise<void> {
  if (inTauri) await invoke("open_url", { url });
  else window.open(url, "_blank", "noopener,noreferrer");
}

/** Whether the host can show a folder of the backend's machine in the file manager. */
export const canShowFolder = inTauri;

/** Shows a folder (a path of the backend's machine) in the file manager. Rejects with a message if it cannot. */
export async function showFolder(path: string): Promise<void> {
  await invoke("show_folder", { path });
}

/** The system's double-click time in milliseconds, from the mouse settings; none in a plain browser, which cannot tell. */
export async function systemDoubleClickMs(): Promise<number | undefined> {
  return inTauri ? invoke<number>("double_click_ms") : undefined;
}

/** The native file dialog; `null` if cancelled. Rejects with a message for a file the backend cannot see. */
export async function pickFile(): Promise<string | null> {
  return inTauri ? invoke<string | null>("pick_file") : null;
}
