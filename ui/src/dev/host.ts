// What the host (Tauri, or a plain browser during development) tells the UI.

export interface BenchConfig {
  /** Run the benchmark on startup, save the results and exit. */
  auto: boolean;
  scrollSeconds: number;
  /** "wsl-split" when the host runs the backend in WSL, else "native". */
  topology: string;
}

const inTauri = "__TAURI_INTERNALS__" in window;

/** Tauri: `VET_BENCH=1`, `VET_BENCH_SCROLL_SECONDS`. Browser: `#bench=1&scroll=20`. */
export async function benchConfig(): Promise<BenchConfig> {
  if (inTauri) {
    const { invoke } = await import("@tauri-apps/api/core");
    return invoke<BenchConfig>("bench_config");
  }
  const params = new URLSearchParams(location.hash.slice(1));
  return { auto: params.get("bench") === "1", scrollSeconds: Number(params.get("scroll")) || 20, topology: "native" };
}

/** Memory in MB of the webview's process tree; null where the host cannot read it. */
export async function rendererMemoryMb(): Promise<number | null> {
  if (!inTauri) return null;
  const { invoke } = await import("@tauri-apps/api/core");
  const bytes = await invoke<number | null>("renderer_memory");
  return bytes === null ? null : bytes / 1048576;
}

export async function saveResults(markdown: string, json: string, ok: boolean): Promise<void> {
  if (!inTauri) return;
  const { invoke } = await import("@tauri-apps/api/core");
  await invoke("save_results", { markdown, json, ok });
}
