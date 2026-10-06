<script lang="ts">
  import IconButton from "../../lib/ui/IconButton.svelte";
  import type { IconName } from "../../lib/ui/icons";

  const WINDOWS: { id: string; icon: IconName; label: string }[] = [
    { id: "queue", icon: "queue", label: "Queue" },
    { id: "files", icon: "files", label: "Files" },
    { id: "timeline", icon: "timeline", label: "Timeline" },
    { id: "comments", icon: "comments", label: "Comments" },
  ];

  let { active, onselect }: { active: string; onselect?: (id: string) => void } = $props();
</script>

<nav aria-label="Tool windows">
  {#each WINDOWS as w (w.id)}
    <IconButton icon={w.icon} label={w.label} active={w.id === active} onclick={() => onselect?.(w.id)} />
  {/each}
</nav>

<style>
  nav {
    width: var(--w-rail);
    flex: none;
    display: flex;
    flex-direction: column;
    align-items: center;
    gap: 4px;
    padding: 6px 0;
    box-sizing: border-box;
    background: var(--win);
    border-right: 1px solid var(--ln);
  }
</style>
