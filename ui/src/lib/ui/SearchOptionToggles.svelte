<script lang="ts">
  // Match case, whole word and regular expression, in the end of a query field.
  import { SEARCH_OPTION_TOGGLES, type SearchOptionToggle } from "../search/optionToggles";

  let {
    values,
    title,
    onchange,
  }: {
    values: Record<SearchOptionToggle["key"], boolean>;
    /** The tooltip of a toggle. */
    title: (toggle: SearchOptionToggle) => string;
    /** Flips the option `key`. */
    onchange: (key: SearchOptionToggle["key"]) => void;
  } = $props();
</script>

<span class="toggles">
  {#each SEARCH_OPTION_TOGGLES as t (t.key)}
    <button
      class="flat toggle"
      class:word={t.key === "wholeWord"}
      type="button"
      aria-pressed={values[t.key]}
      title={title(t)}
      onclick={() => onchange(t.key)}>{t.label}</button
    >
  {/each}
</span>

<style>
  .toggles {
    display: inline-flex;
    gap: 1px;
  }
  .toggle {
    width: 22px;
    height: 20px;
    border-radius: 4px;
    color: var(--text-2);
    font: 600 11px var(--font-mono);
  }
  .toggle.word {
    text-decoration: underline;
    text-underline-offset: 2px;
  }
</style>
