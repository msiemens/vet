// A target's program and arguments as one editable line, and back.

const SAFE = /^[\w@%+=:,./-]+$/;

function quote(word: string): string {
  if (word !== "" && SAFE.test(word)) return word;
  return `"${word.replace(/(["\\])/g, "\\$1")}"`;
}

export function formatCommand(program: string, args: string[]): string {
  return [program, ...args].map(quote).join(" ");
}

/**
 * Splits on whitespace; double quotes group (with `\"` and `\\` escapes), single quotes group
 * literally. No shell expansion: the backend runs the program directly.
 */
export function parseCommand(line: string): { program: string; args: string[] } {
  const words: string[] = [];
  let word: string | undefined;
  for (let i = 0; i < line.length; i++) {
    const c = line[i];
    if (c === '"') {
      word ??= "";
      for (i++; i < line.length && line[i] !== '"'; i++) {
        if (line[i] === "\\" && (line[i + 1] === '"' || line[i + 1] === "\\")) i++;
        word += line[i];
      }
    } else if (c === "'") {
      word ??= "";
      for (i++; i < line.length && line[i] !== "'"; i++) word += line[i];
    } else if (/\s/.test(c)) {
      if (word !== undefined) words.push(word);
      word = undefined;
    } else {
      word = (word ?? "") + c;
    }
  }
  if (word !== undefined) words.push(word);
  return { program: words[0] ?? "", args: words.slice(1) };
}
