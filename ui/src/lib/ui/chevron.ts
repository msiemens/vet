/**
 * Whether a click landed on a row's chevron (`Chevron.svelte`), which opens or shuts what the row holds
 * without choosing it. A target without `closest`, such as a text node, is no chevron.
 */
export function onChevron(e: { target: EventTarget | null }): boolean {
  const target = e.target as Partial<Pick<Element, "closest">> | null;
  return typeof target?.closest === "function" && target.closest(".chev") !== null;
}
