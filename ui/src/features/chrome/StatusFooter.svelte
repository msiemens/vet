<script module lang="ts">
  import type { ServerLine } from "../../state/serverStatus";

  export interface FooterInfo {
    /** The commit on view of those the changeset is reviewed by, e.g. `2 / 5`. */
    commit?: string;
    /** Each part is shown once something reports it: the semantic layer, the caret, the file. */
    languageServer?: ServerLine;
    topology?: { label: string; ok: boolean };
    cursor?: string;
    eol?: string;
    encoding?: string;
    /** What the changeset's problems come to; absent while there is nothing to analyse. */
    problems?: Summary;
  }
</script>

<script lang="ts">
  import type { Snippet } from "svelte";
  import Icon from "../../lib/ui/Icon.svelte";
  import SeverityIcon from "../../lib/ui/SeverityIcon.svelte";
  import Spinner from "../../lib/ui/Spinner.svelte";
  import StatusDot from "../../lib/ui/StatusDot.svelte";
  import type { Summary } from "../../state/problemsSummary";

  let {
    info,
    onserver,
    onproblems,
    progress,
  }: {
    info: FooterInfo;
    /** Shows the details of the language servers. */
    onserver?: () => void;
    /** Shows the Problems tool, narrowed to the problems the change introduced. */
    onproblems?: () => void;
    progress?: Snippet;
  } = $props();
</script>

<footer>
  {#if info.commit}<span class="footer-item mono small" title="The commit on view">{info.commit}</span>{/if}
  {#if info.languageServer}
    {@const server = info.languageServer}
    <button class="flat footer-item" type="button" title={server.title ?? "Language servers"} onclick={onserver}
      >{server.name}{#if server.state}<span class="state"> · {server.state}</span>{/if}</button
    >
  {/if}
  {#if info.topology}
    <span class="footer-item"><StatusDot tone={info.topology.ok ? "ok" : "warn"} />{info.topology.label}</span>
  {/if}
  <span class="grow"></span>
  {#if progress}{@render progress()}{/if}
  {#if info.problems}
    {@const problems = info.problems}
    <button class="flat footer-item" type="button" title={problems.title} onclick={onproblems}>
      {#if problems.icon === "busy"}
        <Spinner />
      {:else if problems.icon === "ok"}
        <span class="ok"><Icon name="check" size={13} stroke={1.8} /></span>
      {:else if problems.icon}
        <SeverityIcon severity={problems.icon} />
      {/if}
      {problems.text}
    </button>
  {/if}
  {#if info.cursor}<span class="footer-item mono small">{info.cursor}</span>{/if}
  {#if info.eol}<span class="footer-item">{info.eol}</span>{/if}
  {#if info.encoding}<span class="footer-item">{info.encoding}</span>{/if}
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
  .ok {
    display: inline-flex;
    color: var(--ok);
  }
  .small {
    font-size: var(--fs-mono-s);
  }
  .grow {
    flex: 1;
  }
</style>
