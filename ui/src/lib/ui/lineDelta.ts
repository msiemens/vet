/** The signed counts of a change, with the true minus sign: `["+12", "−3"]`. */
export const deltaParts = (added: number, removed: number): [string, string] => [`+${added}`, `−${removed}`];

/** The counts as one line of text, for a tooltip: `+12 −3`. */
export const deltaText = (added: number, removed: number): string => deltaParts(added, removed).join(" ");
