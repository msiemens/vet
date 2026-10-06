# UI architecture

Svelte 5 + CodeMirror 6, organised by feature under `src/`.

| Folder | Holds |
|---|---|
| `app/` | `App.svelte`: composition only. Wires features together, applies the theme. |
| `lib/theme/` | `tokens.css` (the only place colours and dimensions are defined, light and dark via `light-dark()`), `global.css`, `theme.ts`. |
| `lib/ui/` | Domain-free primitives (Icon, IconButton, Chip, Meter, Popover, SplitButton). Check here before adding one. |
| `features/<domain>/` | One folder per domain: components, its own logic, `mock.ts` for data the protocol does not carry yet, and an `index.ts` as the public entry. |
| `features/review/editor/` | CodeMirror integration: framework-agnostic TS, no Svelte imports. |
| `state/` | View-model stores and `backend.ts`, the typed API over `connection.ts`. No product logic (SPEC I5). |
| `dev/` | Spike benchmark, metrics and HUD, shown behind the "Dev" toggle in the footer. |

`connection.ts` and `generated/` stay at the top level until the protocol work lands.

## Rules

- `features` import `lib/*` and `state`; `lib/*` imports nothing from `features`, `state` or `app`.
- A feature imports another only through its `index.ts`. `app` composes features through snippets and props.
- Components get data from props or stores. Only `state/backend.ts` talks to the connection.
- Components use tokens (`var(--seen)`, `var(--h-row)`), never literal colours or sizes. The editor reads row heights from the tokens at construction.
- One component per file, named by what it is. No `utils`/`helpers`; put a function next to its user, or in `lib/` if shared.

## Where things go

- New screen: `features/<name>/` with an `index.ts`; mount it from `app/App.svelte`.
- New backend request: a method in `state/backend.ts`, then a store field if the UI displays it.
- Placeholder data: `features/<name>/mock.ts`, passed in as props so wiring is a one-line change.
- Colour or size: `lib/theme/tokens.css` (and a `dev` check in both themes).
