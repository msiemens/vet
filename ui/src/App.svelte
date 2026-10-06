<script lang="ts">
  import { onMount } from "svelte";
  import { runBenchmark } from "./bench";
  import { Connection } from "./connection";
  import { type BenchConfig, benchConfig, rendererMemoryMb, saveResults } from "./host";
  import { type Row, cells, formatTable } from "./metrics";
  import { Shell } from "./shell";

  let root: HTMLElement;
  let shell: Shell | undefined;
  let status = $state("connecting…");
  let mode = $state<"unified" | "split">("unified");
  let keystroke = $state(0);
  let hover = $state(0);
  let collisions = $state(0);
  let rows = $state<Row[]>([]);
  let running = $state(false);

  let config: BenchConfig = { auto: false, scrollSeconds: 20, topology: "native" };

  async function benchmark() {
    if (!shell || running) return;
    running = true;
    rows = [];
    const result = await runBenchmark(shell, root, {
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

  onMount(async () => {
    try {
      const connection = await Connection.open();
      shell = new Shell(connection, root, {
        keystroke: (ms) => (keystroke = ms),
        hover: (ms) => (hover = ms),
        collision: (n) => (collisions = n),
      });
      config = await benchConfig();
      const ms = await shell.open(true);
      status = `${shell.name}: ${shell.entries.length} files, ${shell.view!.totalHunks} hunks; first hunk painted in ${ms.toFixed(0)} ms`;
      if (config.auto) await benchmark();
    } catch (error) {
      status = (error as Error).message;
    }
  });

  function setMode(next: "unified" | "split") {
    mode = next;
    shell?.setMode(next);
  }
</script>

<header class="toolbar">
  <strong>Vet</strong>
  <button class:active={mode === "unified"} onclick={() => setMode("unified")}>Unified</button>
  <button class:active={mode === "split"} onclick={() => setMode("split")}>Side by side</button>
  <button onclick={() => shell?.view?.goToHunk(shell.view.current - 1)}>Prev hunk</button>
  <button onclick={() => shell?.view?.goToHunk(shell.view.current + 1)}>Next hunk</button>
  <button disabled={running} onclick={benchmark}>Run benchmark</button>
  <span class="status">{status}</span>
  <span class="hud" title="Live: keydown to paint, hover round trip, Ctrl-Alt bait bindings fired (must stay 0 when typing AltGr characters)">
    key→paint {keystroke.toFixed(1)} ms · hover {hover.toFixed(1)} ms · collisions {collisions}
  </span>
</header>

<div class="scroll" bind:this={root}></div>

{#if rows.length}
  <section class="results">
    <button onclick={() => (rows = [])}>Close</button>
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
  :global(html, body) { margin: 0; height: 100%; font: 13px system-ui, sans-serif; background: #1e1f22; color: #ddd; }
  :global(#app) { height: 100%; display: flex; flex-direction: column; }
  .toolbar { display: flex; gap: 8px; align-items: center; padding: 6px 10px; background: #2b2d30; flex: none; }
  .toolbar button.active { background: #3574f0; color: white; }
  .status { flex: 1; overflow: hidden; text-overflow: ellipsis; white-space: nowrap; opacity: 0.8; }
  .hud { font-family: monospace; opacity: 0.9; }
  .scroll { flex: 1; overflow-y: auto; min-height: 0; }
  .results { position: fixed; inset: 40px 20px 20px 20px; overflow: auto; background: #2b2d30; border: 1px solid #555; padding: 10px; z-index: 10; }
  table { border-collapse: collapse; width: 100%; }
  td, th { padding: 3px 8px; text-align: left; border-bottom: 1px solid #444; }
  .pass td:last-child { color: #5fb865; }
  .fail td:last-child { color: #f75464; }
  .manual td:last-child { color: #e0a030; }
</style>
