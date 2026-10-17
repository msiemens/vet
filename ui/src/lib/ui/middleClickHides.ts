import type { Action } from "svelte/action";

/** A middle click on the node, except on a control in it, calls `hide`. */
export const middleClickHides: Action<HTMLElement, () => void> = (node, hide) => {
  let current = hide;
  // The middle button's mousedown would start autoscroll and its click paste.
  const down = (e: MouseEvent) => {
    if (e.button === 1) e.preventDefault();
  };
  const aux = (e: MouseEvent) => {
    if (e.button !== 1 || (e.target as Element).closest("button, input, select, a")) return;
    e.preventDefault();
    current();
  };
  node.addEventListener("mousedown", down);
  node.addEventListener("auxclick", aux);
  return {
    update(next) {
      current = next;
    },
    destroy() {
      node.removeEventListener("mousedown", down);
      node.removeEventListener("auxclick", aux);
    },
  };
};
