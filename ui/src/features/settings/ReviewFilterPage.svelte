<script lang="ts">
  import type { PatternMatch } from "../../generated/PatternMatch";
  import type { ReviewFilter } from "../../generated/ReviewFilter";
  import { noun } from "../../lib/format";
  import Checkbox from "../../lib/ui/Checkbox.svelte";
  import SettingsSection from "../../lib/ui/SettingsSection.svelte";
  import { opened } from "../../state/opened.svelte";
  import { patternMatches } from "../../state/projectRequests";
  import { Latest } from "../../state/resource";
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
  let matches = $state.raw<{ roots: Map<string, number>; include: PatternMatch[]; exclude: PatternMatch[] }>();
  const asked = new Latest();

  // The backend counts what the filter as drafted matches; an answer to an older draft is dropped.
  $effect(() => {
    // The counts are of the changeset on view, so they are asked again when another revision comes.
    if (opened.session?.layout?.shown.revision === undefined) return;
    const draft = {
      ...filter,
      include: draftPatterns(filter.include, include),
      exclude: draftPatterns(filter.exclude, exclude),
    };
    const isLatest = asked.begin();
    void patternMatches(draft).then((answer) => {
      if (isLatest() && answer) {
        const roots = new Map(draft.roots.map((root, i) => [root, answer.roots[i]]));
        matches = { ...answer, roots };
      }
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
    <RootList roots={filter.roots} counts={matches?.roots} onchange={(roots) => save({ roots })} />
  </SettingsSection>
  <SettingsSection title="Only review files matching">
    <PatternList
      label="Only review files matching"
      placeholder="e.g. *.rs or ui/src/**"
      patterns={filter.include}
      bind:draft={include}
      matches={matches?.include}
      onchange={(patterns) => save({ include: patterns })}
    />
  </SettingsSection>
  <SettingsSection title="Skip files matching">
    <PatternList
      label="Skip files matching"
      placeholder="e.g. **/*.snap"
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
