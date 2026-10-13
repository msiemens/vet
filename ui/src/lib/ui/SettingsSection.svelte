<!-- A titled section of a settings page: the title sits on a rule that runs on to the right edge. -->
<script lang="ts">
  import type { Snippet } from "svelte";

  let { title, children }: { title: string; children: Snippet } = $props();
</script>

<section aria-label={title}>
  <h3>{title}</h3>
  {@render children()}
</section>

<style>
  section {
    display: flex;
    flex-direction: column;
    gap: 8px;
    margin-top: 20px;
  }
  section:first-child {
    margin-top: 0;
  }
  h3 {
    display: flex;
    align-items: center;
    gap: 8px;
    margin: 0;
    font-size: var(--fs-ctl);
    font-weight: 600;
    color: var(--text);
  }
  h3::after {
    content: "";
    flex: 1;
    height: 1px;
    background: var(--line);
  }
</style>
