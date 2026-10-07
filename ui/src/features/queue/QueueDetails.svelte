<script lang="ts">
  import type { FactGroup } from "./details";

  let { groups }: { groups: FactGroup[] } = $props();
</script>

<div class="groups">
  {#each groups as group (group.title)}
    <section>
      <h3>{group.title}</h3>
      <dl>
        {#each group.facts as fact (fact.label ?? fact.value)}
          {#if fact.label}
            <dt>{fact.label}</dt>
            <dd class="mono">{fact.value}</dd>
          {:else}
            <dd class="statement">{fact.value}</dd>
          {/if}
        {/each}
      </dl>
    </section>
  {/each}
</div>

<style>
  .groups {
    display: flex;
    flex-wrap: wrap;
    gap: 12px 40px;
    padding: 10px 16px;
  }
  h3 {
    margin: 0 0 4px;
    font-size: var(--fs-ctl);
    font-weight: 600;
  }
  dl {
    display: grid;
    grid-template-columns: auto auto;
    gap: 3px 16px;
    margin: 0;
    font-size: var(--fs-ctl);
  }
  dt {
    color: var(--text-2);
  }
  dd {
    margin: 0;
    font-size: var(--fs-mono);
  }
  .statement {
    grid-column: 1 / -1;
    font-family: inherit;
    font-size: var(--fs-ctl);
    color: var(--text-2);
  }
</style>
