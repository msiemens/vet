<script lang="ts">
  import type { SearchFile } from "../../generated/SearchFile";
  import type { SearchLine } from "../../generated/SearchLine";
  import type { View } from "../../generated/View";
  import { segments } from "../../lib/foundText";
  import { basename, dirname } from "../../lib/paths";
  import { NO_HIGHLIGHTS, type Highlights, decodeHighlights } from "../../lib/highlights";
  import FileIcon from "../../lib/ui/FileIcon.svelte";
  import Meter from "../../lib/ui/Meter.svelte";
  import type { Backend } from "../../state/backend";
  import { type PreviewRow, previewRows } from "./searchModel";

  let {
    backend,
    file,
    line,
    view,
    revision,
    repository,
    status,
    letter,
  }: {
    backend: Backend;
    /** The file as a search sees it; `lines` are the matches the preview marks. */
    file: Pick<SearchFile, "path" | "status" | "completion" | "hunks" | "states" | "lines">;
    line: Pick<SearchLine, "kind" | "line">;
    /** The view the file's text is read in. */
    view: View;
    revision: number;
    /** The search was of the target alone: removed lines are not drawn. */
    repository: boolean;
    /** The letter of the file's status; none for an unchanged file. */
    status?: string;
    letter?: string;
  } = $props();

  /** Lines of context on each side of the result. */
  const SPAN = 4;

  interface Texts {
    key: string;
    base: string;
    target: string;
    baseHighlights: Highlights;
    targetHighlights: Highlights;
  }

  let texts = $state.raw<Texts>();
  let failed = $state(false);
  let asked = "";

  const wanted = $derived(`${revision}:${JSON.stringify(view)}:${file.path}`);

  // The texts of the file are read once for all the results in it.
  $effect(() => {
    const key = wanted;
    if (key === asked) return;
    asked = key;
    failed = false;
    const changed = file.status !== undefined;
    const read = changed
      ? backend.fileContents(file.path, view).then((r): Texts => ({
          key,
          base: r.base ?? "",
          target: r.target ?? "",
          baseHighlights: decodeHighlights(r.baseHighlights),
          targetHighlights: decodeHighlights(r.targetHighlights),
        }))
      : backend.fileText(file.path, view).then(({ content }): Texts => {
          if (content.type !== "text") throw new Error("no text");
          return {
            key,
            base: "",
            target: content.text,
            baseHighlights: NO_HIGHLIGHTS,
            targetHighlights: decodeHighlights(content.highlights),
          };
        });
    read.then(
      (loaded) => asked === key && (texts = loaded),
      () => asked === key && ((failed = true), (texts = undefined)),
    );
  });

  const rows = $derived(
    texts?.key === wanted
      ? previewRows({
          base: texts.base,
          target: texts.target,
          hunks: file.hunks,
          kind: line.kind,
          line: line.line,
          span: SPAN,
          withBase: !repository,
        })
      : [],
  );

  /** The matches of the lines of the file the search found, by the side and line they are of. */
  const hitsOf = $derived(
    new Map(file.lines.map((l) => [`${l.kind === "removed" ? "b" : "t"}${l.line}`, l.hits] as const)),
  );

  const lineNo = (row: PreviewRow) => (row.side === "base" ? (row.oldNo ?? 1) - 1 : (row.newNo ?? 1) - 1);

  function parts(row: PreviewRow) {
    const highlights = row.side === "base" ? texts?.baseHighlights : texts?.targetHighlights;
    const hits = hitsOf.get(`${row.side === "base" ? "b" : "t"}${lineNo(row)}`) ?? [];
    return segments(row.text, row.start, highlights ?? NO_HIGHLIGHTS, hits, row.at ? 0 : undefined);
  }

  const dir = $derived(dirname(file.path));
  const stateOf = (hunk: number | undefined) => (hunk === undefined ? undefined : file.states[hunk]);
</script>

<div class="preview">
  <div class="head">
    <FileIcon name={basename(file.path)} size={13} />
    <b>{basename(file.path)}</b>
    {#if dir}<span class="dim">{dir}/</span>{/if}
    <span class="grow"></span>
    <span class="dim">{line.kind === "removed" ? "base line" : "line"} {line.line + 1}</span>
    {#if file.completion && letter}
      <Meter
        width={30}
        title="{Math.round(file.completion.seenPercent)}% seen"
        segments={[
          { value: file.completion.seenPercent, color: "var(--seen)" },
          { value: file.completion.oldBasePercent, color: "var(--old-base-stripes)" },
        ]}
      />
      <span class="mono letter" title={status}>{letter}</span>
    {:else}
      <span class="dim small">unchanged</span>
    {/if}
  </div>
  <div class="code">
    {#each rows as row, i (i)}
      <div class="ln {row.kind}" class:at={row.at} data-state={stateOf(row.hunk)}>
        <span class="no">{row.oldNo ?? ""}</span>
        <span class="no">{row.newNo ?? ""}</span>
        <span class="sign">{row.kind === "added" ? "+" : row.kind === "removed" ? "−" : ""}</span>
        <span class="tx"
          >{#each parts(row) as p, j (j)}{#if p.find}<mark class={[p.className, p.find]}>{p.text}</mark
              >{:else if p.className}<span class={p.className}>{p.text}</span>{:else}{p.text}{/if}{/each}</span
        >
      </div>
    {/each}
    {#if !rows.length}
      <p class="note">{failed ? "The file's text is not available." : "Reading the file…"}</p>
    {/if}
  </div>
</div>

<style>
  .preview {
    flex: none;
    border-top: 1px solid var(--line);
    padding: 4px 0;
    min-height: calc(var(--h-line) * 9 + 30px);
    box-sizing: border-box;
  }
  .head {
    display: flex;
    align-items: center;
    gap: 8px;
    height: 24px;
    padding: 0 12px;
    margin-bottom: 2px;
    border-bottom: 1px solid var(--line-soft);
    color: var(--text-2);
    font-size: 12px;
  }
  .head b {
    color: var(--text);
    font-weight: 600;
  }
  .dim {
    color: var(--text-3);
    font-size: 11.5px;
  }
  .small {
    font-size: 11px;
  }
  .grow {
    flex: 1;
  }
  .letter {
    width: 12px;
    text-align: center;
    color: var(--old-base);
  }
  .ln {
    position: relative;
    display: flex;
    align-items: center;
    height: var(--h-line);
    font: 12px var(--font-mono);
    font-variant-ligatures: none;
    white-space: pre;
    background: var(--bg-editor);
  }
  .ln.added {
    background: var(--add-bg);
  }
  .ln.removed {
    background: var(--del-bg);
  }
  .ln.at {
    box-shadow: inset 0 0 0 1px color-mix(in srgb, var(--accent) 55%, transparent);
  }
  .ln[data-state="seen"]::before,
  .ln[data-state="seenAgainstOldBase"]::before {
    content: "";
    position: absolute;
    left: 0;
    top: 0;
    bottom: 0;
    width: 3px;
    background: var(--seen);
  }
  .ln[data-state="seenAgainstOldBase"]::before {
    background: var(--old-base-stripes);
  }
  .no {
    width: 40px;
    flex: none;
    box-sizing: border-box;
    padding-right: 10px;
    text-align: right;
    color: var(--text-3);
  }
  .sign {
    width: 14px;
    flex: none;
    color: var(--text-3);
  }
  .added .sign {
    color: var(--add-text);
  }
  .removed .sign {
    color: var(--del-text);
  }
  mark {
    background: var(--find);
    color: inherit;
    border-radius: 2px;
  }
  mark.current {
    background: var(--find-current);
    box-shadow: 0 0 0 1px var(--find-current-ring);
  }
  .note {
    margin: 8px 12px;
    color: var(--text-3);
    font-size: 12px;
  }
</style>
