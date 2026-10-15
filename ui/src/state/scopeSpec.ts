// Which view a tab reads: the changeset as a whole, or one of its commits by full id. A tab's view
// scope is keyed by this, so the same view is made once.

/** The changeset as a whole, or the commit with the full id. */
export type ScopeSpec = { type: "changeset" } | { type: "commit"; id: string };

export const scopeKey = (spec: ScopeSpec): string => (spec.type === "changeset" ? "changeset" : spec.id);
