<script module lang="ts">
  export interface FooterInfo {
    /** Settled snapshot id and its age. */
    snapshot: string;
    languageServer: string;
    topology: { label: string; ok: boolean };
    cursor: string;
    eol: string;
    encoding: string;
    diagnostics: number;
  }
</script>

<script lang="ts">
  import type { Snippet } from "svelte";

  let { info, end }: { info: FooterInfo; end?: Snippet } = $props();
</script>

<footer>
  <span class="mono small">{info.snapshot}</span>
  <span>{info.languageServer}</span>
  <span class="topology"><span class="dot" class:ok={info.topology.ok}></span>{info.topology.label}</span>
  <span class="grow"></span>
  <span title="Diagnostics">{info.diagnostics} diagnostics</span>
  <span class="mono small">{info.cursor}</span>
  <span>{info.eol}</span>
  <span>{info.encoding}</span>
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
    border-top: 1px solid var(--ln);
    background: var(--win);
    color: var(--tx2);
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
