<script module lang="ts">
  export interface MenuItem {
    id: string;
    label: string;
    detail?: string;
  }
</script>

<script lang="ts">
  let { items, onselect }: { items: MenuItem[]; onselect: (id: string) => void } = $props();
</script>

<ul role="menu">
  {#each items as item (item.id)}
    <li role="none">
      <button class="flat" role="menuitem" type="button" onclick={() => onselect(item.id)}>
        <span>{item.label}</span>
        {#if item.detail}<span class="mono detail">{item.detail}</span>{/if}
      </button>
    </li>
  {/each}
</ul>

<style>
  ul {
    list-style: none;
    margin: 0;
    padding: 4px 0;
  }
  button {
    width: 100%;
    height: var(--h-row);
    display: flex;
    align-items: center;
    justify-content: space-between;
    gap: 16px;
    padding: 0 12px;
    font-size: var(--fs-ctl);
    white-space: nowrap;
  }
  .detail {
    font-size: var(--fs-mono-s);
    color: var(--text-3);
  }
</style>
