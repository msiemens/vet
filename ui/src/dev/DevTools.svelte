<script lang="ts">
  import type { ReviewController } from "../features/review";
  import type { Theme } from "../lib/theme/theme";
  import { runBenchmark } from "./bench";
  import { hud } from "./hud.svelte";
  import { type BenchConfig, benchConfig, rendererMemoryMb, saveResults } from "./host";
  import { type Row, cells, formatTable } from "./metrics";

  let {
    controller,
    open = $bindable(false),
    status = $bindable(""),
    theme,
    ontheme,
  }: { controller?: ReviewController; open?: boolean; status?: string; theme: Theme; ontheme: (t: Theme) => void } = $props();

  let rows = $state<Row[]>([]);
  let running = $state(false);

  async function benchmark(config: BenchConfig) {
    if (!controller || running) return;
    running = true;
    rows = [];
    const result = await runBenchmark(controller, controller.root, {
      scrollSeconds: config.scrollSeconds,
      samples: 200,
      topology: config.topology,
      memoryMb: rendererMemoryMb,
      progress: (m) => (status = `benchmark: ${m}`),
    });
    rows = result;
    const markdown = `Topology: ${config.topology}\n\n${formatTable(result)}`;
    console.log(markdown);
    status = "benchmark done";
    running = false;
    const json = JSON.stringify({ topology: config.topology, rows: result }, null, 2);
    await saveResults(markdown, json, result.every((r) => r.pass !== false)).catch((e) => console.error(e));
  }

  /** Runs the benchmark, and shows the bar, if the host asked for it at startup. */
  export async function runIfRequested() {
    const config = await benchConfig();
    if (!config.auto) return;
    open = true;
    await benchmark(config);
  }
</script>

{#if open}
  <div class="bar">
    <strong>Dev</strong>
    <button type="button" disabled={running} onclick={async () => benchmark(await benchConfig())}>Run benchmark</button>
    <select value={theme} onchange={(e) => ontheme(e.currentTarget.value as Theme)} aria-label="Theme">
      <option value="system">System theme</option>
      <option value="light">Light</option>
      <option value="dark">Dark</option>
    </select>
    <span class="status">{status}</span>
    <span class="hud mono" title="Live: keydown to paint, hover round trip, Ctrl-Alt bait bindings fired (must stay 0 when typing AltGr characters)">
      key→paint {hud.keystroke.toFixed(1)} ms · hover {hud.hover.toFixed(1)} ms · collisions {hud.collisions}
    </span>
  </div>
{/if}

{#if rows.length}
  <section class="results">
    <button type="button" onclick={() => (rows = [])}>Close</button>
    <table>
      <thead><tr><th>Metric</th><th>p95</th><th>Threshold</th><th>n</th><th>Result</th></tr></thead>
      <tbody>
        {#each rows as r}
          <tr class={r.pass === null ? "manual" : r.pass ? "pass" : "fail"}>
            {#each cells(r) as cell}<td>{cell}</td>{/each}
          </tr>
        {/each}
      </tbody>
    </table>
  </section>
{/if}

<style>
  .bar { display: flex; gap: 8px; align-items: center; padding: 4px 10px; background: var(--hunk); border-top: 1px solid var(--ln); font-size: 12px; flex: none; }
  .status { flex: 1; overflow: hidden; text-overflow: ellipsis; white-space: nowrap; color: var(--tx2); }
  .hud { color: var(--tx2); }
  .results { position: fixed; inset: 40px 20px 20px 20px; overflow: auto; background: var(--sheet); border: 1px solid var(--ln); padding: 10px; z-index: 30; }
  table { border-collapse: collapse; width: 100%; }
  td, th { padding: 3px 8px; text-align: left; border-bottom: 1px solid var(--ln2); }
  .pass td:last-child { color: var(--ok); }
  .fail td:last-child { color: var(--delt); }
  .manual td:last-child { color: var(--old); }
</style>
