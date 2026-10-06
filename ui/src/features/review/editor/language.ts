// Syntax highlighting by file extension; unknown types stay plain. Colours are theme tokens.

import { css } from "@codemirror/lang-css";
import { html } from "@codemirror/lang-html";
import { javascript } from "@codemirror/lang-javascript";
import { json } from "@codemirror/lang-json";
import { markdown } from "@codemirror/lang-markdown";
import { python } from "@codemirror/lang-python";
import { rust } from "@codemirror/lang-rust";
import { yaml } from "@codemirror/lang-yaml";
import { HighlightStyle, StreamLanguage, syntaxHighlighting } from "@codemirror/language";
import { shell } from "@codemirror/legacy-modes/mode/shell";
import { toml } from "@codemirror/legacy-modes/mode/toml";
import type { Extension } from "@codemirror/state";
import { tags as t } from "@lezer/highlight";

function language(path: string): Extension {
  const ext = path.slice(path.lastIndexOf(".") + 1).toLowerCase();
  switch (ext) {
    case "rs":
      return rust();
    case "py":
    case "pyi":
      return python();
    case "ts":
    case "mts":
    case "cts":
      return javascript({ typescript: true });
    case "tsx":
      return javascript({ typescript: true, jsx: true });
    case "js":
    case "mjs":
    case "cjs":
      return javascript();
    case "jsx":
      return javascript({ jsx: true });
    case "json":
      return json();
    case "md":
    case "markdown":
      return markdown();
    case "yaml":
    case "yml":
      return yaml();
    case "toml":
      return StreamLanguage.define(toml);
    case "sh":
    case "bash":
      return StreamLanguage.define(shell);
    case "css":
      return css();
    case "html":
    case "htm":
    case "svg":
      return html();
    default:
      return [];
  }
}

const highlightStyle = HighlightStyle.define([
  { tag: [t.keyword, t.operatorKeyword, t.modifier], color: "var(--syn-keyword)" },
  { tag: [t.string, t.special(t.string)], color: "var(--syn-string)" },
  { tag: [t.number, t.bool, t.null], color: "var(--syn-number)" },
  { tag: [t.comment, t.docComment], color: "var(--text-3)", fontStyle: "italic" },
  { tag: [t.typeName, t.className, t.namespace], color: "var(--syn-type)" },
  { tag: [t.function(t.variableName), t.function(t.propertyName), t.macroName], color: "var(--syn-function)" },
  { tag: [t.attributeName, t.meta, t.annotation, t.tagName], color: "var(--syn-number)" },
  { tag: [t.heading, t.strong], fontWeight: "bold", color: "var(--syn-function)" },
  { tag: [t.link, t.url], color: "var(--accent)" },
]);

export const syntax = (path: string): Extension => [language(path), syntaxHighlighting(highlightStyle)];
