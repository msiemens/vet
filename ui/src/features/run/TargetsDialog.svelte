<script lang="ts">
  import type { RunTarget } from "../../generated/RunTarget";
  import Dialog from "../../lib/ui/Dialog.svelte";
  import { runs } from "../../state/runs.svelte";
  import { formatCommand, parseCommand, sourceOf } from "./command";

  interface Draft {
    label: string;
    command: string;
    cwd: string;
    env: string;
  }

  const draftOf = (t: RunTarget): Draft => ({
    label: t.label,
    command: formatCommand(t.program, t.args),
    cwd: t.cwd,
    env: t.env.map((e) => `${e.name}=${e.value}`).join("\n"),
  });
  const empty: Draft = { label: "", command: "", cwd: "", env: "" };

  /** The target being edited; a new one has an id the backend has not seen. */
  let id = $state<string>();
  let draft = $state<Draft>(structuredClone(empty));
  let busy = $state(false);

  const entry = $derived(runs.targets.find((t) => t.target.id === id));
  const original = $derived(entry ? draftOf(entry.target) : empty);
  const dirty = $derived(
    draft.label !== original.label ||
      draft.command !== original.command ||
      draft.cwd !== original.cwd ||
      draft.env !== original.env,
  );
  const env = $derived(
    draft.env
      .split("\n")
      .map((l) => l.trim())
      .filter(Boolean)
      .map((l) => ({ line: l, at: l.indexOf("=") })),
  );
  const problem = $derived.by(() => {
    if (!draft.label.trim()) return "A target needs a name.";
    if (!parseCommand(draft.command).program) return "A target needs a command.";
    const bad = env.find((e) => e.at < 1);
    return bad ? `Not NAME=value: ${bad.line}` : undefined;
  });
  const custom = $derived(id !== undefined && sourceOf(id) === "custom");

  select(runs.primary?.target.id);

  function select(next: string | undefined) {
    id = next;
    const target = runs.targets.find((t) => t.target.id === next)?.target;
    draft = structuredClone(target ? draftOf(target) : empty);
  }

  function add() {
    id = `custom:${Date.now().toString(36)}`;
    draft = structuredClone(empty);
  }

  async function save() {
    if (id === undefined || problem) return;
    const { program, args } = parseCommand(draft.command);
    const target: RunTarget = {
      id,
      label: draft.label.trim(),
      program,
      args,
      cwd: draft.cwd.trim(),
      env: env.map((e) => ({ name: e.line.slice(0, e.at), value: e.line.slice(e.at + 1) })),
    };
    busy = true;
    try {
      await runs.save(target);
    } finally {
      busy = false;
    }
  }

  async function remove() {
    if (id === undefined) return;
    busy = true;
    try {
      if (await runs.remove(id)) select(runs.targets[0]?.target.id);
    } finally {
      busy = false;
    }
  }
</script>

<Dialog title="Run targets" width="720px" onclose={() => (runs.editing = false)}>
  <div class="cols">
    <div class="side">
      <div class="list" role="listbox" aria-label="Targets">
        {#each runs.targets as { target, stored } (target.id)}
          <button
            type="button"
            class="flat item"
            role="option"
            aria-selected={target.id === id}
            class:on={target.id === id}
            onclick={() => select(target.id)}
          >
            <span class="name">{target.label}</span>
            <span class="mono src">{stored ? "edited · " : ""}{sourceOf(target.id)}</span>
          </button>
        {/each}
        {#if id !== undefined && !entry}
          <button type="button" class="flat item on" role="option" aria-selected="true">
            <span class="name">{draft.label || "New target"}</span>
            <span class="mono src">new</span>
          </button>
        {/if}
      </div>
      <button type="button" class="flat add" onclick={add}>+ Add custom target</button>
    </div>

    {#if id === undefined}
      <p class="hint">No targets were found in the manifests. Add one to run it from here.</p>
    {:else}
      <form
        onsubmit={(e) => {
          e.preventDefault();
          void save();
        }}
      >
        <label>
          <span>Name</span>
          <input bind:value={draft.label} />
        </label>
        <label>
          <span>Command</span>
          <input class="mono" bind:value={draft.command} spellcheck="false" />
        </label>
        <label>
          <span>Working directory <em>relative to the repository root, empty for the root</em></span>
          <input class="mono" bind:value={draft.cwd} spellcheck="false" />
        </label>
        <label>
          <span>Environment <em>one NAME=value per line</em></span>
          <textarea class="mono" rows="3" bind:value={draft.env} spellcheck="false"></textarea>
        </label>
        {#if dirty && problem}<p class="problem">{problem}</p>{/if}
        <button type="submit" hidden aria-label="Save"></button>
      </form>
    {/if}
  </div>

  {#snippet actions()}
    {#if entry?.stored}
      <button class="ctl" type="button" disabled={busy} onclick={remove}
        >{custom ? "Delete" : "Reset to discovered"}</button
      >
    {/if}
    <span class="grow"></span>
    <button class="ctl" type="button" onclick={() => (runs.editing = false)}>Close</button>
    <button class="ctl primary" type="button" disabled={busy || !dirty || !!problem} onclick={save}>Save</button>
  {/snippet}
</Dialog>

<style>
  .cols {
    display: flex;
    gap: 16px;
    min-height: 240px;
  }
  .side {
    flex: none;
    width: 230px;
    display: flex;
    flex-direction: column;
    gap: 6px;
  }
  .list {
    display: flex;
    flex-direction: column;
    border: 1px solid var(--line);
    border-radius: 6px;
    background: var(--bg-editor);
    overflow: auto;
    max-height: 300px;
  }
  .item {
    display: flex;
    flex-direction: column;
    align-items: flex-start;
    gap: 1px;
    padding: 4px 10px;
    text-align: left;
    font-size: var(--fs-ctl);
    flex: none;
  }
  .name {
    max-width: 100%;
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }
  .src {
    font-size: var(--fs-mono-s);
    color: var(--text-3);
  }
  .add {
    height: var(--h-ctl);
    padding: 0 10px;
    text-align: left;
    font-size: var(--fs-ctl);
    color: var(--accent);
  }
  .list .item.on {
    color: var(--text);
  }
  form {
    flex: 1;
    min-width: 0;
    display: flex;
    flex-direction: column;
    gap: 10px;
    font-size: var(--fs-ctl);
  }
  label {
    display: flex;
    flex-direction: column;
    gap: 4px;
    color: var(--text-2);
  }
  em {
    font-style: normal;
    color: var(--text-3);
  }
  input,
  textarea {
    border: 1px solid var(--line);
    border-radius: var(--radius);
    background: var(--bg-editor);
    color: var(--text);
    font-size: var(--fs-mono);
    padding: 0 8px;
    box-sizing: border-box;
  }
  input {
    height: var(--h-ctl);
  }
  input:not(.mono) {
    font: inherit;
    font-size: var(--fs-ctl);
  }
  textarea {
    padding: 4px 8px;
    resize: vertical;
  }
  .problem,
  .hint {
    margin: 0;
    font-size: 12px;
  }
  .problem {
    color: var(--warn);
  }
  .hint {
    color: var(--text-3);
  }
  .grow {
    flex: 1;
  }
</style>
