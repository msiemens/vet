<script module lang="ts">
  export interface FooterInfo {
    /** The range of versions the review shows, e.g. `v14 → v17`. */
    range?: string;
    /** Each part is shown once something reports it: the semantic layer, the caret, the file. */
    languageServer?: string;
    /** Why, as a tooltip of `languageServer`. */
    languageServerTitle?: string;
    topology?: { label: string; ok: boolean };
    cursor?: string;
    eol?: string;
    encoding?: string;
    diagnostics?: number;
  }
</script>

<script lang="ts">
  import type { Snippet } from "svelte";

  let { info, progress, end }: { info: FooterInfo; progress?: Snippet; end?: Snippet } = $props();
</script>

<footer>
  {#if info.range}<span class="mono small" title="The versions the review shows">{info.range}</span>{/if}
  {#if info.languageServer}<span title={info.languageServerTitle}>{info.languageServer}</span>{/if}
  {#if info.topology}
    <span class="topology"><span class="dot" class:ok={info.topology.ok}></span>{info.topology.label}</span>
  {/if}
  <span class="grow"></span>
  {#if progress}{@render progress()}{/if}
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
