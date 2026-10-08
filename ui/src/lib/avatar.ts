/** Which of the six avatar colours (1-6) a name gets, the same every time. */
export function avatarColor(name: string): number {
  let hash = 0;
  for (const c of name) hash = (hash * 31 + c.charCodeAt(0)) >>> 0;
  return (hash % 6) + 1;
}
