<script lang="ts">
  import { onMount } from "svelte";
  import tile from "../../../src-tauri/icons/app-macos.svg?no-inline";
  import {
    type StartupProgress,
    appVersion,
    onStartupProgress,
    quit,
    retryStartup,
    startupProgress,
  } from "../../state/host";
  import { describe } from "./startup";

  let progress = $state<StartupProgress>();
  let version = $state("");
  const view = $derived(progress && describe(progress));

  onMount(() => {
    const unlisten = onStartupProgress((next) => (progress = next));
    // The host may have reported steps before this page listened; an event that arrives first is newer.
    void unlisten.then(startupProgress).then((current) => (progress ??= current));
    void appVersion().then((v) => (version = v));
    return () => void unlisten.then((stop) => stop());
  });
</script>

<main>
  <header>
    <img class="tile" src={tile} alt="" />
    <div>
      <h1>Vet <span class="version mono">{version}</span></h1>
      <p>Your Integrated Review Environment</p>
    </div>
  </header>

  <footer>
    {#if view}
      <div
        class="bar"
        role="progressbar"
        aria-label="Startup"
        aria-valuemin={0}
        aria-valuemax={view.total}
        aria-valuenow={view.done}
      >
        <div class="done" class:failed={view.error} style:width="{(view.done / view.total) * 100}%"></div>
        {#if !view.error}
          <div class="step" style:left="{(view.done / view.total) * 100}%" style:width="{100 / view.total}%"></div>
        {/if}
      </div>
      {#if view.error}
        <p class="error selectable" role="alert">{view.error}</p>
        <div class="actions">
          <button class="ctl primary" onclick={() => void retryStartup()}>Retry</button>
          <button class="ctl" onclick={() => void quit()}>Quit</button>
        </div>
      {:else}
        <p class="status">
          {view.label}{#if view.target}<span class="target mono">{view.target}</span>{/if}
        </p>
      {/if}
    {/if}
  </footer>
</main>

<style>
  main {
    box-sizing: border-box;
    flex: 1;
    display: flex;
    flex-direction: column;
    justify-content: space-between;
    padding: 36px;
    background: var(--bg-editor);
    border: 1px solid var(--line);
  }
  header {
    flex: 1;
    display: flex;
    align-items: center;
    gap: 22px;
  }
  /* The icon's plate sits in a margin of its canvas; the margin is cropped by the negative space. */
  .tile {
    width: 108px;
    height: 108px;
    margin: -12px;
    flex: none;
  }
  h1 {
    margin: 0;
    font-size: 30px;
    font-weight: 600;
    line-height: 1.2;
  }
  .version {
    margin-left: 6px;
    font-size: var(--fs-mono-s);
    font-weight: 400;
    color: var(--text-3);
  }
  header p {
    margin: 4px 0 0;
    color: var(--text-2);
  }
  footer {
    min-height: 72px;
  }
  .bar {
    position: relative;
    height: 4px;
    overflow: hidden;
    background: var(--track);
  }
  .done {
    height: 100%;
    background: var(--accent);
  }
  .done.failed {
    background: var(--warn);
  }
  /* The current step's share of the bar: its length is unknown, so it only shows activity. */
  .step {
    position: absolute;
    top: 0;
    bottom: 0;
    overflow: hidden;
  }
  .step::before {
    content: "";
    position: absolute;
    inset: 0;
    width: 40%;
    background: var(--accent);
    opacity: 0.6;
    animation: sweep 1.4s ease-in-out infinite;
  }
  @keyframes sweep {
    from {
      transform: translateX(-100%);
    }
    to {
      transform: translateX(250%);
    }
  }
  @media (prefers-reduced-motion: reduce) {
    .step::before {
      animation: none;
      width: 100%;
    }
  }
  .status,
  .error {
    margin: 12px 0 0;
    font-size: var(--fs-ctl);
    color: var(--text-2);
  }
  .target {
    margin-left: 10px;
    font-size: var(--fs-mono-s);
    color: var(--text-3);
  }
  .error {
    color: var(--warn);
    overflow-wrap: anywhere;
    max-height: 4.5em;
    overflow-y: auto;
  }
  .actions {
    display: flex;
    justify-content: flex-end;
    gap: 8px;
    margin-top: 10px;
  }
</style>
