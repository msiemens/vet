/** `one` for 1, else `many` (the plural by an added `s` unless given). */
export const noun = (n: number, one: string, many = `${one}s`): string => (n === 1 ? one : many);

/** A count with its noun: "1 file", "2 files". */
export const plural = (n: number, one: string, many?: string): string => `${n} ${noun(n, one, many)}`;
