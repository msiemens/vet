<script lang="ts">
  // A recent project in a list: its tile, its name and path with the matches of the search marked, and
  // what the list adds at the end. `roomy` is the welcome screen's size.
  import type { Snippet } from "svelte";
  import Marked from "../../lib/ui/Marked.svelte";
  import PathText from "../../lib/ui/PathText.svelte";
  import ProjectAvatar from "./ProjectAvatar.svelte";
  import type { RecentMatch } from "./recentList";

  let { match, roomy = false, trailing }: { match: RecentMatch; roomy?: boolean; trailing?: Snippet } = $props();
</script>

<span class="row" class:roomy title={match.project.path}>
  <ProjectAvatar path={match.project.path} />
  <span class="text">
    <span class="name"><Marked text={match.name} ranges={match.nameRanges} /></span>
    <span class="mono path"><PathText path={match.project.path} ranges={match.pathRanges} /></span>
  </span>
  {@render trailing?.()}
</span>

<style>
  .row {
    display: flex;
    align-items: center;
    gap: 10px;
    height: var(--h-switcher-row);
    padding: 0 8px;
  }
  .roomy {
    gap: 12px;
    height: var(--h-welcome-row);
    padding: 0 12px;
  }
  .text {
    flex: 1;
    min-width: 0;
    display: flex;
    flex-direction: column;
  }
  .name {
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
    font-size: var(--fs-ctl);
  }
  .roomy .name {
    font-size: var(--fs);
  }
  .path {
    display: flex;
    font-size: var(--fs-mono-s);
  }
</style>
