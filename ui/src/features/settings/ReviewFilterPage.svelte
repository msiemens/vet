<script lang="ts">
  import type { PatternMatch } from "../../generated/PatternMatch";
  import type { ReviewFilter } from "../../generated/ReviewFilter";
  import { noun } from "../../lib/format";
  import Checkbox from "../../lib/ui/Checkbox.svelte";
  import SettingsSection from "../../lib/ui/SettingsSection.svelte";
  import { changeset } from "../../state/changeset.svelte";
  import PatternList from "./PatternList.svelte";
  import RootList from "./RootList.svelte";
  import { type PatternDraft, draftPatterns } from "./filterDraft";

  let {
    filter,
    excluded,
    onsave,
  }: {
    /** The repository's filter, as the manifest carries it. */
    filter: ReviewFilter;
    /** Changes the filter currently excludes; the backend computes them. */
    excluded: { lines: number; items: number };
    /** Saves a changed filter; resolves with the backend's message if it refused it. */
    onsave: (filter: ReviewFilter) => Promise<string | undefined>;
  } = $props();

  let error = $state<string>();
  let include = $state<PatternDraft>({ edits: {}, adding: "" });
  let exclude = $state<PatternDraft>({ edits: {}, adding: "" });
  let matches = $state.raw<{ include: PatternMatch[]; exclude: PatternMatch[] }>();
  let asked = 0;

  // The backend counts what the filter as drafted matches; an answer to an older draft is dropped.
  $effect(() => {
    // The counts are of the changeset on view, so they are asked again when another revision comes.
    if (changeset.layout?.shown.revision === undefined) return;
    const draft = {
      ...filter,
      include: draftPatterns(filter.include, include),
      exclude: draftPatterns(filter.exclude, exclude),
    };
    const n = ++asked;
    void changeset.patternMatches(draft).then((answer) => {
      if (n === asked && answer) matches = answer;
    });
  });

  /** A refused edit stays in its field with the message below; an accepted one comes back as the new manifest's filter. */
  async function save(change: Partial<ReviewFilter>): Promise<string | undefined> {
    error = await onsave({ ...filter, ...change });
    return error;
  }
</script>

<div class="page">
  <SettingsSection title="Review roots">
    <RootList roots={filter.roots} onchange={(roots) => save({ roots })} />
  </SettingsSection>
  <SettingsSection title="Only review files matching">
    <PatternList
      name="include"
      label="Include"
      patterns={filter.include}
      bind:draft={include}
      matches={matches?.include}
      onchange={(patterns) => save({ include: patterns })}
    />
  </SettingsSection>
  <SettingsSection title="Skip files matching">
    <PatternList
      name="exclude"
      label="Exclude"
      patterns={filter.exclude}
      bind:draft={exclude}
      matches={matches?.exclude}
      onchange={(patterns) => save({ exclude: patterns })}
    />
  </SettingsSection>
  <Checkbox checked={filter.hideBinary} onchange={(hideBinary) => void save({ hideBinary })}>Hide binary files</Checkbox
  >
  <Checkbox checked={filter.reviewGenerated} onchange={(reviewGenerated) => void save({ reviewGenerated })}
    >Review lockfiles and generated files</Checkbox
  >
  {#if error}<p class="err" role="alert">{error}</p>{/if}
  <p class="count">
    <span class="mono">{excluded.lines}</span> changed {noun(excluded.lines, "line")} and
    <span class="mono">{excluded.items}</span> other {noun(excluded.items, "change")} excluded
  </p>
</div>

<style>
  .page {
    display: flex;
    flex-direction: column;
    gap: 10px;
    font-size: var(--fs-ctl);
  }
  p {
    margin: 0;
  }
  .mono {
    font-family: var(--font-mono);
    font-size: var(--fs-mono);
  }
  .count {
    padding-top: 8px;
    border-top: 1px solid var(--line-soft);
    color: var(--text-2);
  }
</style>
