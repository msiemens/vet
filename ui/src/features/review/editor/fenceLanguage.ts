// Which language a Markdown code fence is in, as a file extension `language.ts` understands.

const BY_NAME: Record<string, string> = {
  rust: "rs",
  rs: "rs",
  python: "py",
  py: "py",
  typescript: "ts",
  ts: "ts",
  tsx: "tsx",
  javascript: "js",
  js: "js",
  jsx: "jsx",
  csharp: "cs",
  cs: "cs",
  "c#": "cs",
  json: "json",
  yaml: "yaml",
  yml: "yaml",
  toml: "toml",
  sh: "sh",
  bash: "sh",
  shell: "sh",
  zsh: "sh",
  css: "css",
  html: "html",
  markdown: "md",
  md: "md",
};

/**
 * The extension for a fence's info string (`rust`, `rust,ignore`, `ts {1}`). A fence without a
 * language takes the hovered file's (servers leave it out for the file's own language);
 * an unknown name is plain text.
 */
export function fenceExtension(info: string, path: string): string | undefined {
  const name = info
    .trim()
    .split(/[\s,{]/, 1)[0]
    .toLowerCase();
  if (!name) return path;
  return BY_NAME[name];
}
