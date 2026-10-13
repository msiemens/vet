<script lang="ts">
  import Icon from "../../lib/ui/Icon.svelte";
  import StatusDot from "../../lib/ui/StatusDot.svelte";
  import type { Action, EmptyView, Row } from "./emptyChanges";

  let {
    view,
    onact,
  }: {
    view: EmptyView;
    /** Carries out what a row, a link or a button asks. */
    onact: (action: Action) => void;
  } = $props();
</script>

{#snippet value(row: Row)}
  {#if row.live}<StatusDot tone="ok" />{/if}
  {#each row.value as seg, i (i)}
    {#if typeof seg === "string"}{seg}{:else}<span class="mono code">{seg.mono}</span>{/if}
  {/each}
{/snippet}

{#snippet cells(row: Row)}
  {#if row.label !== undefined}<span class="label">{row.label}</span>{/if}
  <span class="value">{@render value(row)}</span>
  <span class="meta">{row.meta ?? ""}</span>
  {#if row.action}
    {@const action = row.action}
    <button class="link" type="button" onclick={() => onact(action)}>{action.label}</button>
  {:else}
    <span></span>
  {/if}
{/snippet}

<div class="wrap">
  <div class="block" role="status">
    <span class="glyph"><Icon name={view.glyph} size={24} stroke={1.5} /></span>
    <div class="title">{view.title}</div>
    <div class="lead">{view.lead}</div>
    {#each view.panels as panel, p (p)}
      <div class="panel">
        {#if panel.heading}
          <div class="head">
            <span>{panel.heading}</span>
            {#if panel.count !== undefined}<span class="mono n">{panel.count}</span>{/if}
            {#if panel.link}
              {@const link = panel.link}
              <button class="link end" type="button" onclick={() => onact(link)}>{link.label}</button>
            {/if}
          </div>
        {/if}
        {#each panel.rows as row, r (r)}
          {#if row.open}
            {@const open = row.open}
            <button
              class="flat row"
              class:nolabel={row.label === undefined}
              type="button"
              title={open.path}
              onclick={() => onact(open)}>{@render cells(row)}</button
            >
          {:else}
            <div class="row" class:nolabel={row.label === undefined}>{@render cells(row)}</div>
          {/if}
        {/each}
      </div>
    {/each}
    {#if view.actions.length}
      <div class="actions">
        {#each view.actions as action (action.id + action.label)}
          <button class="ctl" class:primary={action.primary} type="button" onclick={() => onact(action)}>
            {action.label}
          </button>
        {/each}
      </div>
    {/if}
  </div>
</div>

<style>
  .wrap {
    position: absolute;
    inset: 0;
    z-index: 1;
    display: flex;
    justify-content: center;
    align-items: flex-start;
    padding: clamp(14px, 6%, 40px) 24px 24px;
    overflow: auto;
  }
  .block {
    display: flex;
    flex-direction: column;
    align-items: center;
    gap: 8px;
    width: min(100%, 540px);
  }
  .glyph {
    display: inline-flex;
    margin-bottom: 2px;
    color: var(--text-3);
  }
  .title {
    font-size: 15px;
    font-weight: 600;
    text-align: center;
    text-wrap: balance;
  }
  .lead {
    max-width: 46ch;
    color: var(--text-2);
    line-height: 1.5;
    text-align: center;
    text-wrap: pretty;
  }
  .panel {
    width: 100%;
    box-sizing: border-box;
    margin-top: 8px;
    border: 1px solid var(--line);
    border-radius: 6px;
    background: var(--bg-editor);
    overflow: hidden;
    text-align: left;
    user-select: text;
  }
  .head {
    display: flex;
    align-items: center;
    gap: 8px;
    height: 26px;
    padding: 0 10px;
    background: var(--bg-panel);
    border-top: 1px solid var(--line-soft);
    font-size: 11.5px;
    text-transform: uppercase;
    letter-spacing: 0.06em;
    font-weight: 600;
    color: var(--text-3);
  }
  .panel > .head:first-child {
    border-top: 0;
  }
  .n {
    font-weight: 400;
    letter-spacing: 0;
  }
  .end {
    margin-left: auto;
    text-transform: none;
    letter-spacing: 0;
    font-weight: 400;
  }
  .row {
    display: grid;
    grid-template-columns: 94px minmax(0, 1fr) auto auto;
    gap: 10px;
    align-items: center;
    min-height: 26px;
    width: 100%;
    box-sizing: border-box;
    padding: 2px 10px;
    border-top: 1px solid var(--line-soft);
    font-size: var(--fs-ctl);
    text-align: left;
  }
  .row.nolabel {
    grid-template-columns: minmax(0, 1fr) auto auto;
  }
  .head + .row,
  .panel > .row:first-child {
    border-top: 0;
  }
  .label {
    color: var(--text-3);
    font-size: 11.5px;
  }
  .value {
    min-width: 0;
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }
  .meta {
    color: var(--text-3);
    font-size: 11.5px;
    white-space: nowrap;
  }
  .code {
    color: var(--text-2);
  }
  .value > .code:first-of-type {
    color: var(--text);
  }
  .value > :global(.dot) {
    margin-right: 6px;
  }
  .actions {
    display: flex;
    flex-wrap: wrap;
    justify-content: center;
    gap: 8px;
    margin-top: 10px;
  }
</style>
