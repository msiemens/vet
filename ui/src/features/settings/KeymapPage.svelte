<script lang="ts">
  import { bindingOf, specOf } from "../../lib/keymap";
  import { conflictsOf } from "../../lib/keymapEdit";
  import SearchField from "../../lib/ui/SearchField.svelte";
  import SettingsSection from "../../lib/ui/SettingsSection.svelte";
  import { commands } from "../../state/commands";
  import { isMac } from "../../state/host";
  import { keymap } from "../../state/keymap.svelte";
  import { keymapGroups } from "./keymapRows";

  let query = $state("");
  /** The command whose new key is being read from the keyboard. */
  let recording = $state<string>();
  /** A key that another command has: waits for the viewer to take it over or give it up. */
  let clash = $state<{ id: string; spec: string; key: string; others: string[] }>();
  let problem = $state<string>();

  const listing = $derived(commands.list());
  const groups = $derived(keymapGroups(listing, keymap.overrides, query));
  const anyChanged = $derived(Object.keys(keymap.overrides).length > 0);

  // While a key is being read, every key goes here and runs nothing.
  $effect(() => {
    const id = recording;
    if (id === undefined) return;
    return commands.capture((e) => {
      if (e.key === "Escape" && !e.ctrlKey && !e.altKey && !e.shiftKey && !e.metaKey) {
        recording = undefined;
        return;
      }
      const spec = specOf(e);
      if (spec === null) {
        if (!["Control", "Alt", "Shift", "Meta"].includes(e.key)) {
          problem = "That key combination types a character or cannot be a shortcut. Press another one.";
        }
        return;
      }
      recording = undefined;
      const others = conflictsOf(listing, id, spec);
      if (others.length === 0) keymap.bind(id, spec);
      else
        clash = { id, spec, key: bindingOf(spec, isMac ? "Meta" : "Ctrl").label, others: others.map((c) => c.label) };
    });
  });

  function record(id: string) {
    clash = undefined;
    problem = undefined;
    recording = id;
  }
</script>

<div class="bar">
  <SearchField bind:value={query} label="Search commands and keys" placeholder="Search commands and keys" />
  <button class="ctl" type="button" disabled={!anyChanged} onclick={() => keymap.resetAll()}>Reset all</button>
</div>

{#if problem}<p class="problem">{problem}</p>{/if}

<div>
  {#each groups as group (group.area)}
    <SettingsSection title={group.label}>
      <ul>
        {#each group.rows as row (row.id)}
          <li data-command={row.id}>
            <span class="label">{row.label}</span>
            {#if recording === row.id}
              <span class="hint">Press a key combination, Esc to cancel</span>
              <button class="ctl" type="button" onclick={() => (recording = undefined)}>Cancel</button>
            {:else}
              {#if row.binding}
                <kbd class:changed={row.changed}>{row.binding.label}</kbd>
              {:else}
                <span class="none">{row.changed ? "Removed" : "None"}</span>
              {/if}
              {#each row.alsoBindings as also (also.spec)}
                <kbd>{also.label}</kbd>
              {/each}
              <span class="actions">
                <button class="ctl" type="button" onclick={() => record(row.id)}>Change</button>
                <button class="ctl" type="button" disabled={!row.binding} onclick={() => keymap.unbind(row.id)}>
                  Remove
                </button>
                <button class="ctl" type="button" disabled={!row.changed} onclick={() => keymap.reset(row.id)}>
                  Reset
                </button>
              </span>
            {/if}
            {#if row.sharedWith.length}
              <p class="problem">Also used by {row.sharedWith.join(", ")}.</p>
            {/if}
            {#if clash?.id === row.id}
              <p class="problem">
                <kbd>{clash.key}</kbd> is already used by {clash.others.join(", ")}.
                <button
                  class="ctl"
                  type="button"
                  onclick={() => {
                    if (clash) keymap.bind(clash.id, clash.spec);
                    clash = undefined;
                  }}>Take it over</button
                >
                <button class="ctl" type="button" onclick={() => (clash = undefined)}>Keep as is</button>
              </p>
            {/if}
          </li>
        {/each}
      </ul>
    </SettingsSection>
  {:else}
    <p class="empty">No command matches.</p>
  {/each}
</div>

<style>
  .bar {
    display: flex;
    gap: 8px;
    margin-bottom: 12px;
  }
  ul {
    list-style: none;
    margin: 0;
    padding: 0;
  }
  li {
    display: flex;
    flex-wrap: wrap;
    align-items: center;
    gap: 4px 10px;
    padding: 4px 0;
    border-bottom: 1px solid var(--line-soft);
    font-size: var(--fs-ctl);
  }
  .label {
    flex: 1;
    min-width: 0;
  }
  kbd {
    font-family: var(--font-mono);
    font-size: var(--fs-mono-s);
    padding: 1px 6px;
    border: 1px solid var(--line);
    border-radius: var(--radius);
  }
  kbd.changed {
    border-color: var(--accent);
  }
  .none,
  .hint {
    color: var(--text-3);
    font-size: var(--fs-mono);
  }
  .actions {
    display: flex;
    gap: 4px;
  }
  .problem {
    flex-basis: 100%;
    margin: 0;
    color: var(--del-text);
    font-size: var(--fs-mono);
  }
  .empty {
    color: var(--text-3);
  }
</style>
