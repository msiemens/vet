// The Tauri host's commands for choosing and remembering repositories. A plain browser
// (development only) has none: no dialog, and nothing is remembered.

const inTauri = "__TAURI_INTERNALS__" in window;

/** Whether a native folder dialog exists. */
export const canPickFolder = inTauri;

async function invoke<T>(command: string, args?: Record<string, unknown>): Promise<T> {
  const { invoke } = await import("@tauri-apps/api/core");
  return invoke<T>(command, args);
}

/** The last folder of a backend-side path, for display. */
export const repoName = (path: string): string => path.split(/[/\\]/).filter(Boolean).at(-1) ?? path;

/** The native folder dialog; `null` if cancelled. Rejects with a message for a folder the backend cannot open. */
export async function pickRepository(): Promise<string | null> {
  return inTauri ? invoke<string | null>("pick_repository") : null;
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
