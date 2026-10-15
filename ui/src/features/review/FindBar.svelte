<script lang="ts">
  import { plural } from "../../lib/format";
  import { SEARCH_OPTION_TOGGLES } from "../../lib/search/optionToggles";
  import { changeGate } from "../../lib/ui/changeGate";
  import FilterChip from "../../lib/ui/FilterChip.svelte";
  import IconButton from "../../lib/ui/IconButton.svelte";
  import Icon from "../../lib/ui/Icon.svelte";
  import SearchOptionToggles from "../../lib/ui/SearchOptionToggles.svelte";
  import Segmented from "../../lib/ui/Segmented.svelte";
  import Spinner from "../../lib/ui/Spinner.svelte";
  import { commands } from "../../state/commands";
  import { isMac } from "../../state/host";
  import { openFindInFiles } from "../search";
  import type { FindOptions, FindSide } from "./editor/findMatches";
  import type { FindState } from "./FindState.svelte";

  let {
    find,
    fileView,
    unchanged,
    onoption,
    onstep,
    onclose,
  }: {
    find: FindState;
    /** The tab shows the file's text: it has one side, and its unchanged lines are not set apart. */
    fileView: boolean;
    /** The file has no changes: no sides to choose, no changed lines to narrow to. */
    unchanged: boolean;
    onoption: (patch: Partial<FindOptions>) => void;
    onstep: (delta: 1 | -1) => void;
    onclose: () => void;
  } = $props();

  let input = $state<HTMLInputElement>();
  const summary = $derived(find.summary);
  const options = $derived(find.options);
  const noSides = $derived(fileView || unchanged);

  // The cursor goes to the field each time the bar is asked for, also when it is open already.
  const focusRequested = changeGate();
  $effect(() => {
    if (focusRequested(find.focusRequests)) {
      input?.focus();
      input?.select();
    }
  });

  const count = $derived.by(() => {
    if (summary.error !== undefined) return { text: "Invalid pattern", tone: "bad" };
    if (!options.text) return { text: "Type to find", tone: "dim" };
    if (!summary.total)
      return { text: summary.pending ? "Searching…" : "No results", tone: summary.pending ? "dim" : "bad" };
    const more = summary.pending ? "+" : "";
    return {
      text: summary.index < 0 ? `${summary.total}${more} results` : `${summary.index + 1} of ${summary.total}${more}`,
      tone: "",
    };
  });

  const countTitle = $derived(
    summary.error ??
      (summary.skipped
        ? `${plural(summary.skipped, "file is", "files are")} over the size limit and not searched`
        : undefined),
  );

  const sides: { id: FindSide; label: string }[] = [
    { id: "both", label: "Both" },
    { id: "target", label: "Target" },
    { id: "base", label: "Base" },
  ];
  const sideOptions = $derived(
    sides.map((s) => ({
      ...s,
      disabled: noSides,
      title: unchanged
        ? "This file has no changes"
        : fileView
          ? "The File view shows the target's text only"
          : undefined,
    })),
  );

  /** Escape closes the bar from any of its controls. */
  function escape(e: KeyboardEvent) {
    if (e.key !== "Escape") return;
    e.preventDefault();
    onclose();
  }

  function keydown(e: KeyboardEvent) {
    if (e.key === "Enter") {
      e.preventDefault();
      onstep(e.shiftKey ? -1 : 1);
    } else if (e.altKey && !e.ctrlKey && !e.shiftKey && e.metaKey === isMac) {
      // Option types a letter in a field, so the toggles are read by the key's place.
      const toggle = SEARCH_OPTION_TOGGLES.find((t) => t.code === e.code);
      if (!toggle) return;
      e.preventDefault();
      onoption({ [toggle.key]: !options[toggle.key] });
    }
  }
</script>

<!-- The handler only hears keys of the controls inside. -->
<!-- svelte-ignore a11y_no_noninteractive_element_interactions -->
<div class="bar" role="search" onkeydown={escape}>
  <span class="glyph"><Icon name="search" size={14} /></span>
  <div class="query" class:bad={summary.error !== undefined}>
    <input
      bind:this={input}
      type="text"
      value={options.text}
      placeholder="Find"
      aria-label="Find"
      spellcheck="false"
      autocomplete="off"
      oninput={(e) => onoption({ text: e.currentTarget.value })}
      onkeydown={keydown}
    />
    <SearchOptionToggles
      values={options}
      title={(t) =>
        t.key === "regex"
          ? `${commands.tooltip(t.id, t.name)}. JavaScript syntax here; Find in Files uses Rust's`
          : commands.tooltip(t.id, t.name)}
      onchange={(key) => onoption({ [key]: !options[key] })}
    />
  </div>
  <span class="count {count.tone}" title={countTitle}>{count.text}</span>
  {#if summary.pending}<Spinner label="Reading the texts of {summary.pending} more files" />{/if}
  {#if summary.total && !options.onlyChanges && summary.unchanged && !noSides}
    <span class="sub" title="Matches in lines the change did not touch, inside collapsed regions or in context"
      >{summary.unchanged} in unchanged code</span
    >
  {/if}
  <IconButton
    icon="up"
    label="Previous match"
    title={commands.tooltip("find.previous", "Previous match")}
    disabled={!summary.total}
    size="m"
    iconSize={14}
    onclick={() => onstep(-1)}
  />
  <IconButton
    icon="down"
    label="Next match"
    title={commands.tooltip("find.next", "Next match")}
    disabled={!summary.total}
    size="m"
    iconSize={14}
    onclick={() => onstep(1)}
  />
  <span class="sep"></span>
  <Segmented
    small
    label="Side"
    value={noSides ? "target" : options.side}
    onchange={(side) => onoption({ side })}
    options={sideOptions}
  />
  <FilterChip
    on={options.onlyChanges}
    disabled={unchanged}
    title="Only lines the change added or removed"
    onclick={() => onoption({ onlyChanges: !options.onlyChanges })}>Only changes</FilterChip
  >
  <span class="grow"></span>
  <IconButton
    icon="tree"
    label="Find in Files"
    title={commands.tooltip("find.inFiles", "Find in Files with this query")}
    disabled={!commands.isEnabled("find.inFiles")}
    size="m"
    iconSize={14}
    onclick={() =>
      openFindInFiles({
        query: options.text,
        matchCase: options.matchCase,
        wholeWord: options.wholeWord,
        regex: options.regex,
      })}
  />
  <IconButton icon="close" label="Close" title="Close (Esc)" size="m" iconSize={13} onclick={onclose} />
</div>

<style>
  .bar {
    display: flex;
    align-items: center;
    gap: 8px;
    height: var(--h-bar);
    box-sizing: border-box;
    flex: none;
    min-width: 0;
    padding: 0 8px 0 10px;
    background: var(--bg-panel);
    border-bottom: 1px solid var(--line);
    font-size: var(--fs-ctl);
    white-space: nowrap;
  }
  .glyph {
    display: inline-flex;
    color: var(--text-3);
  }
  .query {
    display: flex;
    align-items: center;
    gap: 4px;
    flex: 0 1 250px;
    min-width: 120px;
    height: var(--h-ctl);
    box-sizing: border-box;
    padding: 0 3px 0 8px;
    border: 1px solid var(--line);
    border-radius: var(--radius);
    background: var(--bg-control);
  }
  .query:focus-within {
    outline: var(--focus-width) solid var(--focus);
    outline-offset: -1px;
    border-color: transparent;
  }
  .query.bad {
    border-color: var(--del-text);
  }
  input {
    flex: 1;
    min-width: 0;
    padding: 0;
    border: 0;
    outline: none;
    background: transparent;
    color: var(--text);
    font: var(--fs-mono) var(--font-mono);
    font-variant-ligatures: none;
  }
  input::placeholder {
    color: var(--text-3);
    font-family: var(--font-ui);
  }
  .count {
    min-width: 60px;
    font-variant-numeric: tabular-nums;
  }
  .count.dim {
    color: var(--text-3);
  }
  .count.bad {
    color: var(--del-text);
  }
  .sub {
    color: var(--text-3);
    font-size: var(--fs-mono-s);
  }
  .sep {
    width: 1px;
    height: var(--h-ctl-xxs);
    background: var(--line);
    flex: none;
  }
  .grow {
    flex: 1;
  }
</style>
