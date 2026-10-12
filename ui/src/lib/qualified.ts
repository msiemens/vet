// A qualified name (`Acme.Billing.Core`, `crate::billing::core`) as two parts, for `.path-text`.

const OPEN = "<([{";
const CLOSE = ">)]}";

/**
 * The qualifier of a qualified name with its trailing separator (`.` or `::`), and its last
 * segment. A separator inside brackets belongs to a type argument or a parameter list
 * (`List<Foo.Bar>`, `Merge(A.B)`), so it does not split. A name with a space outside brackets
 * (`impl From<A> for B`) is a description, not a path of segments, and stays whole.
 */
export function splitQualified(name: string): { qualifier: string; name: string } {
  let depth = 0;
  let cut = 0;
  for (let i = 0; i < name.length; i++) {
    const c = name[i];
    if (OPEN.includes(c)) depth++;
    else if (CLOSE.includes(c)) depth = Math.max(0, depth - 1);
    else if (depth > 0) continue;
    else if (/\s/.test(c)) return { qualifier: "", name };
    else if (c === ".") cut = i + 1;
    else if (c === ":" && name[i + 1] === ":") cut = ++i + 1;
  }
  return { qualifier: name.slice(0, cut), name: name.slice(cut) };
}
