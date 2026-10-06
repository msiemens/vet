<script module lang="ts">
  export interface FooterInfo {
    /** Settled snapshot id and its age. */
    snapshot: string;
    /** The rest is shown once something reports it: the semantic layer, the caret, the file. */
    languageServer?: string;
    topology?: { label: string; ok: boolean };
    cursor?: string;
    eol?: string;
    encoding?: string;
    diagnostics?: number;
  }
</script>

<script lang="ts">
  import type { Snippet } from "svelte";

  let { info, end }: { info: FooterInfo; end?: Snippet } = $props();
</script>

<footer>
  <span class="mono small">{info.snapshot}</span>
  {#if info.languageServer}<span>{info.languageServer}</span>{/if}
  {#if info.topology}
    <span class="topology"><span class="dot" class:ok={info.topology.ok}></span>{info.topology.label}</span>
  {/if}
  <span class="grow"></span>
  {#if info.diagnostics !== undefined}<span title="Diagnostics">{info.diagnostics} diagnostics</span>{/if}
  {#if info.cursor}<span class="mono small">{info.cursor}</span>{/if}
  {#if info.eol}<span>{info.eol}</span>{/if}
  {#if info.encoding}<span>{info.encoding}</span>{/if}
  {#if end}{@render end()}{/if}
</footer>

<style>
  footer {
    display: flex;
    flex-wrap: wrap;
    gap: 16px;
    align-items: center;
    min-height: var(--h-footer);
    padding: 0 10px;
    box-sizing: border-box;
    border-top: 1px solid var(--line);
    background: var(--bg-window);
    color: var(--text-2);
    font-size: 12px;
    flex: none;
  }
  .small {
    font-size: var(--fs-mono-s);
  }
  .grow {
    flex: 1;
  }
  .topology {
    display: inline-flex;
    align-items: center;
    gap: 6px;
  }
  .dot {
    width: 6px;
    height: 6px;
    border-radius: 50%;
    background: var(--warn);
  }
  .dot.ok {
    background: var(--ok);
  }
</style>
