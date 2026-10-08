<script module lang="ts">
  import type { ServerLine } from "../../state/serverStatus";

  export interface FooterInfo {
    /** The range of versions the review shows, e.g. `v14 → v17`. */
    range?: string;
    /** The commit on view of those the changeset is reviewed by, e.g. `2 / 5`. */
    commit?: string;
    /** Each part is shown once something reports it: the semantic layer, the caret, the file. */
    languageServer?: ServerLine;
    topology?: { label: string; ok: boolean };
    cursor?: string;
    eol?: string;
    encoding?: string;
    diagnostics?: number;
  }
</script>

<script lang="ts">
  import type { Snippet } from "svelte";

  let {
    info,
    onserver,
    progress,
    end,
  }: {
    info: FooterInfo;
    /** Shows the details of the language servers. */
    onserver?: () => void;
    progress?: Snippet;
    end?: Snippet;
  } = $props();
</script>

<footer>
  {#if info.range}<span class="footer-item mono small" title="The versions the review shows">{info.range}</span>{/if}
  {#if info.commit}<span class="footer-item mono small" title="The commit on view">{info.commit}</span>{/if}
  {#if info.languageServer}
    {@const server = info.languageServer}
    <button class="flat footer-item" type="button" title={server.title ?? "Language servers"} onclick={onserver}
      >{server.name}{#if server.state}<span class="state"> · {server.state}</span>{/if}</button
    >
  {/if}
  {#if info.topology}
    <span class="footer-item"><span class="dot" class:ok={info.topology.ok}></span>{info.topology.label}</span>
  {/if}
  <span class="grow"></span>
  {#if progress}{@render progress()}{/if}
  {#if info.diagnostics !== undefined}<span class="footer-item" title="Diagnostics">{info.diagnostics} diagnostics</span
    >{/if}
  {#if info.cursor}<span class="footer-item mono small">{info.cursor}</span>{/if}
  {#if info.eol}<span class="footer-item">{info.eol}</span>{/if}
  {#if info.encoding}<span class="footer-item">{info.encoding}</span>{/if}
  {#if end}{@render end()}{/if}
</footer>

<style>
  footer {
    display: flex;
    flex-wrap: wrap;
    align-items: stretch;
    min-height: var(--h-footer);
    box-sizing: border-box;
    border-top: 1px solid var(--line);
    background: var(--bg-window);
    color: var(--text-2);
    font-size: 12px;
    flex: none;
  }
  .state {
    color: var(--text-3);
  }
  .small {
    font-size: var(--fs-mono-s);
  }
  .grow {
    flex: 1;
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
