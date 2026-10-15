/**
 * Answers whether the values it is given differ from the ones it last answered for, comparing by
 * reference; the first call always differs. A component calls it inside an effect with the values
 * the effect depends on, so the reads stay plain expressions and the effect can act on a change.
 */
export function changeGate(): (...values: unknown[]) => boolean {
  let seen: unknown[] | undefined;
  return (...values) => {
    const previous = seen;
    seen = values;
    if (previous === undefined) return true;
    if (previous.length !== values.length) return true;
    return !values.every((v, i) => Object.is(v, previous[i]));
  };
}
