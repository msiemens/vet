# Vet — Spec

This file describes what Vet does and the rules it must always follow, not how the window looks or where its controls are. How Vet is built is in `ARCHITECTURE.md` (backend) and `ui/ARCHITECTURE.md` (UI).

In this file, the **current changeset** is the one changeset of a project that Vet has open at a time. The **review screen** shows its changes; the **queue** lists changesets.

## 1. Product

Vet is a desktop app for reviewing code changes, in particular changes made by coding agents. It shows changesets rather than files, and the reviewer reads and marks code rather than writing it. Agents run outside Vet.

What it does, in order of priority:

1. Review changesets: uncommitted changes, local commits, branches, pull requests.
2. Answer questions about the code: navigation, impact, reachability, the old version against the new one.
3. Comment, with comments synced to GitHub or Azure DevOps.

**Non-goals:**

- Writing code or running agents.
- Integrations with specific agent tools, or guessing which tool made a change.
- Being a general-purpose IDE.
- Collaboration between several users. Vet is a personal tool.

**Threat model:** agents are assumed to make mistakes, not to attack. Vet does not defend against malicious code, with one exception: code from outside your own project. That is pull requests from a fork and, in a project you have not marked trusted, all pull requests. Vet does not run anything that executes such code until you trust it (§6.5).

**Platforms.** The backend, the language servers, Vet's storage and the repository under review always run on the same machine, the _backend side_. Only the window (the shell) may run elsewhere.

| Setup     | Window           | Backend side                             | Status    |
| --------- | ---------------- | ---------------------------------------- | --------- |
| macOS     | Tauri            | macOS                                    | supported |
| Windows   | Tauri            | Windows                                  | supported |
| WSL split | Tauri on Windows | WSL2, repository on the Linux filesystem | supported |

## 2. Invariants

### I1 Cache keys cover every input

The key of a cached language-server answer holds:

- for an answer about one file: the blob id of that file's text;
- for an answer that looks across files: the snapshot, the analysis policy (§6.5) and the dependency restore;
- always: the server's fingerprint (binary, version, settings, toolchain, and any user configuration outside the repository that it reads).

Further rules:

- No time-based or event-based invalidation: a cached answer is valid exactly while its key matches.
- For the working tree, which changes under the server, only per-file answers are cached.
- Caches are disposable: deleting one only makes Vet slower.
- Vet computes the server's fingerprint and the restore's state once, and again after a launch, a settings change, a restore, a server restart or clearing the caches.

Tests:

- For each kind of cached answer, changing any one input is a cache miss, and changing none is a hit.
- The same scenario gives the same results with all caches off and with caching on.
- Partial answers (§6.3) are never stored.
- Deleting the cache file and the materialized snapshots (§6.2) changes no review state and no result.

### I2 Review state is anchored to content, not line numbers

When the code changes, comments and review marks are remapped, and each ends up `Exact`, `Inferred`, `Stale` or `Ambiguous`. "Seen" stays only on content that is identical to what was reviewed: new or changed content is never marked seen without the reviewer doing it.

- Test: comments survive amend, rebase, force-push, renames, moves and agent edits. They are remapped with the confidence recorded, and never silently lost.
- Test: when new edits split a seen region, the identical parts stay seen and the rest becomes unseen.

### I3 Facts about code come only from language servers

Types, references, diagnostics and calls come only from language servers; Vet does not guess them. Vet may compute further results from them (_derived_ results), but labels them as derived and shows what they were computed from.

- Test: every fact can be traced to a language server, and every derived result shows its inputs.
- Test: an operation a server does not support is reported as "not supported by this language's server", never as an empty result. A server that cannot receive file contents is refused with a visible error.

## 3. Concepts

- **Snapshot:** the contents of a working tree at one moment: every file with its content, mode and kind (file, symlink or submodule). It never changes.
- **Edit timeline:** the settled snapshots of something that changes over time (the working tree, a branch, a pull request), in order.
- **Changeset:** a base snapshot and a target snapshot.
- **Interdiff:** a diff of two diffs: how the changes of one version of a changeset differ from the changes of another. Since it compares changes rather than files, a rebase alone gives an empty interdiff.
- **Review filter:** which files to review (review roots, include and exclude patterns), set per repository. It is applied when a changeset is shown and is never part of a snapshot.
- **Pin:** the commit and tree recorded when a review starts (version 1), with the uncommitted content of that snapshot copied into the content store.
- **Anchor:** where a comment or mark is attached: one range of content on one side (I2).
- **Thread:** a comment thread on a line range, a file or the whole changeset, optionally linked to a thread on the platform.
- **Reviewed span:** a range of content the reviewer marked seen or flagged, on one or both sides. Vet stores spans; whether a hunk is seen is worked out from them. A concern is a comment, not a mark.
- **Session:** a working tree against a base: browsing until a review starts (§4.5), then an open review of one changeset with its pin, versions and marks.

**Provenance:** a snapshot holds file contents only. Its origin, its base commit, branch and pull request details, and its place in the timeline are stored separately, by snapshot id.

## 4. Changesets and versions

### 4.1 Snapshots

- **What is in a snapshot:** every file under the repository root that git tracks, plus every untracked file that git does not ignore.
  - Files larger than 50 MB (configurable) are hashed in full, so two snapshots never look equal by mistake, but Vet does not keep their content and shows them as `content unavailable`.
  - Ignored files can be included explicitly.
  - Untracked files in folders that belong to a tool by name (`.pnpm-store/`) are never included. This list is fixed.
  - Vet never guesses which files matter.
  - A change of the executable bit or of a symlink is a change like any other. A submodule is recorded as its commit; Vet does not look inside it unless asked.
- **When a review starts (§4.5)**, Vet records the commit and tree of version 1 and copies its uncommitted and untracked content into Vet's content-addressed store (the _content store_, §9). Vet never writes uncommitted content into git's object database, so untracked secrets never end up in something that could be pushed. The changeset is the base against the latest settled snapshot.
- **Keeping content:**
  - While a review is open, every new snapshot's content that git cannot rebuild (uncommitted and untracked files) is copied into the content store, so every version of the review can be shown again later.
  - Content shown on either side of a diff, or under a comment or a seen mark, is copied when first shown. Review state therefore never depends on git keeping old objects.
  - The working tree's uncommitted content is copied ahead of a review only while the review screen shows it.
- **Session refs:** while a review is open, Vet keeps refs on the commits its snapshots come from, so git's garbage collection never removes them. They are deleted when the review is finished.
- **Materializing a snapshot** (writing it to a directory to run a language server on it) works only while all of its file contents are available, from git or from the content store. If any are missing, the language features for that snapshot are disabled with a visible reason. Vet never starts a server on an incomplete copy.

### 4.2 Watching and settling

- **Watching:** Vet watches the working tree for file changes and re-hashes what changed. Watching only makes Vet faster; reconciliation (below) decides what the snapshot is.
- **Settling:** a burst of writes is debounced: a snapshot is _settled_ once the changed files' size and modification time have stayed the same for about 1–2 seconds. Each settled snapshot is added to the edit timeline. If two writers change the same file within that window, the result is one snapshot, and the two cannot be told apart.
- **Reconciliation:** Vet scans the whole working tree when it opens it, when the watcher reports that it lost events, after a resume or restart, and before a snapshot settles. Lost watcher events can delay review state but never make it wrong. Where the watcher cannot cover the tree, Vet polls instead and shows a notice.
- A working tree that is not the current changeset and has no open review is not scanned. Vet scans it once when it becomes the current changeset.
- A folder Vet cannot read does not stop the scan. Vet skips it with a warning: tracked files in it are taken from git's index and marked as content unavailable, and untracked files in it are left out. A folder that disappears during the scan counts as deleted.

### 4.3 Files as git sees them

- **Git:** Vet reads git inside its own process. It never starts git, and never runs hooks, `fsmonitor` commands, or filter or diff drivers.
- **Paths** are relative to the repository root, use `/`, and are compared by their exact bytes (as git stores them for tracked files, as the filesystem reports them for untracked ones). Unicode normalization is used only for display and to match git's `core.precomposeUnicode` on macOS. Two paths that differ only in case or only in Unicode normalization are flagged as a `PathCollision`; Vet does not materialize such a snapshot on a filesystem that would merge them, and says so.
- **Content:** a file's content is what git would store, after git's built-in conversions only (`eol`, `text`, `core.autocrlf`, `ident`). Otherwise every file under `autocrlf` would look changed. For files with a custom filter (for example Git LFS): if the file's size and date match git's index, Vet uses the index's content; otherwise it uses the bytes on disk and marks them `unfiltered`.
- **Mode and kind:** where the filesystem cannot store them (`core.filemode=false`, `core.symlinks=false`, usual on Windows), they come from git's index for tracked files. Untracked files get mode `100644` and kind `file`.
- **Fetch:** Vet fetches from the remotes inside its own process, read-only, so remote branches and pull requests are current. It never pulls, merges or rebases.
  - What Vet fetches by itself (every few minutes, pull request heads and targets) reaches only remotes on a platform host with an account (§7.1), always over HTTPS with the account's token, whatever URL the remote names. It never runs `ssh` and never asks git's credential helpers, so it never prompts.
  - A fetch the user asks for reaches every remote as git is configured to, so an SSH remote may ask its agent then.

### 4.4 Bases

- **History:** Vet's own timeline of settled snapshots is the record of what happened. The reflog is extra information (which commit a snapshot came from, comparisons across amends and rebases); git may delete it.
- **The base depends on the kind of changeset. The user does not choose it.**
  - A pull request's base is the point where it branched off its target branch, as the platform shows it (the merge base with the fetched target branch).
  - A local branch's base is its merge base with the default branch. A branch built on another branch is also compared with the default branch.
  - The working tree's base is `HEAD` at the moment it was opened. Vet keeps this base across restarts. Commits an agent makes on top do not move it, so the working tree shows everything done since, committed or not. The base stays until the review of the working tree is finished; the next time the working tree is opened, Vet takes the `HEAD` it finds then. Before a review starts, this base is the only thing Vet keeps, so closing and reopening Vet before the first mark does not move it.
- The base is never the tip of the target branch, because that would show what the target gained since as reverse changes. A changeset whose tip the target already contains is empty, and Vet says so (§8).
- A project's changesets are its working tree, its local branches and the open pull requests of its repository. The default branch is not one: the others are compared with it. Comparing two arbitrary commits is not a review and is not offered.
- When a branch's or pull request's merge base moves (a rebase, a merge of the target), the base moves with it, as a new version.
- A changeset's line counts use the same base wherever they are given, so they agree. Until Vet has counted them, a pull request's counts are the platform's own.
- **`HEAD` off the base:** when the working tree's `HEAD` no longer descends from its base (the agent rebased, reset or checked out something else), the changes shown include work from elsewhere. Vet says so, with the base's commit, for as long as this is true.

### 4.5 Reviews and versions

- **Browsing:** opening a repository shows the working tree against its base. Vet records no versions, keeps no content and creates no refs.
- **A review starts** with the first review action: marking something seen (a range, a hunk or a file), acknowledging a non-text change, or adding a comment. Removing a mark and reading do not start a review.
  - The snapshot at that moment is version 1. Vet records its commit and tree, and the base (§4.4) becomes the review's base. Each later settled snapshot is the next version (v2, v3, …).
  - A pull request's versions are the pushes its platform recorded (GitHub's force-pushes, Azure DevOps' iterations), from the first push. A review that starts has one version per push made so far, the current head being the last. Each later push is a new version, however many pushes happened between two fetches. Vet fetches their commits from the platform.
  - A push whose commits the platform no longer has (dropped after a force-push) is still a version, but without content. It is listed and cannot be opened, and the pushes around it are compared with the nearest version that has content.
  - A local branch's versions are the tips Vet saw.
  - Keeping content and session refs (§4.1) apply from the start. A review stays open, and keeps recording versions, after it is complete (§5.3).
  - While a review of the working tree is open, every settled snapshot becomes a version, whichever screen is shown.
- **Finishing a review** is an explicit action, allowed with changes still unseen or hunks still flagged; the reviewer is told how many flagged hunks are left. Vet records whether the review was complete, deletes the session refs and stops recording versions. Later edits are browsing again until the next review action starts a new review. Finished reviews are kept as §9 says.
- **Reopening** a repository with an open review continues it: the working tree settles as the next version and the marks are remapped (§5.2). Without an open review, Vet opens in browsing.
- **Changes between versions:** a version, or a range of versions, can be shown on its own, as an interdiff (§3). The count of files changed since the last review uses the same interdiff, so a change the author undid, and a change that only comes from the base move, are not counted.
- **By commit:** a branch or pull request is shown either as all its changes at once (the default, and what completion counts) or commit by commit. By commit, a clean merge of the target branch is a collapsed marker with nothing to review, and a merge with conflict resolutions shows only the resolutions (as `git show --remerge-diff`).

### 4.6 The review filter

- The review filter decides which files of a changeset are reviewed: review roots, whether binary files are hidden, and include and exclude patterns. It is a project setting, stored per repository and used for every changeset. Changing it never changes snapshots, so it never disturbs comments or marks.
- **Generated files** are excluded like files matching an exclude pattern, unless the filter's "review generated files" option is on. Vet treats these as generated:
  - lockfiles and tool output by file name (`Cargo.lock`, `yarn.lock`, `*.pb.go`, `*.min.js`, …, in any folder);
  - files marked `linguist-generated` in `.gitattributes` (on the side the file is on; `-linguist-generated` keeps a file in);
  - files whose first five lines (within the first KiB) contain `<auto-generated`, `@generated` or Go's `// Code generated … DO NOT EDIT.`.
- **Narrowing:** the current review can be narrowed to one folder instead of changing the filter. The narrowing replaces the project's filter for this review only, is not saved, and ends when the project's filter changes or another changeset becomes current.
- While the filter is edited, the backend gives the files and lines each pattern matches in the current changeset, or why the pattern is invalid, and the number of changes excluded.

## 5. Review state

### 5.1 Anchors and comments

- **What a comment is on:** a line range on one side (an anchor), a whole file on one side, or the whole changeset. Comments are made on the changeset, never on one commit of it.
- **A thread** has its messages (author, text, time; the first opens the thread) and a status in Vet, open or resolved. A thread linked to the platform also shows the platform's own status as the platform names it (for example Azure DevOps' "Won't fix"), whether it can be resolved there (not a comment in a flat conversation), and whether it blocks the merge (§7.2).
- Only comments on a line range are remapped as below, and only they can become `Stale` or `Ambiguous`. A comment on a file follows the file through renames git detects, and becomes `Stale` only when the file is deleted. A comment on the whole changeset never moves.

**Remapping a comment.** A comment is remapped on the same side of the changeset it is shown in, directly from the snapshot it was made in to the snapshot shown, never through the versions in between. The result is one of:

- **`Exact`:** the commented bytes are where the diff maps the range, at the same path or a path git detected as a rename. If only whitespace differs, the comment is exact with "whitespace changed".
- **`Inferred`:** Vet found one clearly best place, by the surrounding lines or by the same content in another changed file (a move), and records how it found it and how sure it is. When the commented lines were rewritten, the comment moves to the lines that replaced them, with low confidence. When they were deleted, it stays where they were, marked "line deleted".
- **`Ambiguous`:** several places are about equally good.
- **`Stale`:** the file was deleted, or none of the surrounding text survived.

Vet never moves a comment without saying so: an `Inferred` comment is marked as moved, with Vet's confidence, how it was found and where it was, until the reviewer confirms it at its new place (→ `Exact`). Comments remapped with low confidence can be listed together.

### 5.2 Seen, flags and remapping

- **A span** covers reviewed lines on the old side, the new side or both (a pure addition has no old side, a pure deletion no new side). It is `Seen`, `SeenAgainstOldBase` or `Flagged`.
- **Marking a hunk seen** (or a selection) stores one span covering its lines on both sides. A hunk shows as seen when every changed line in it is covered by a `Seen` span. Marking seen can be undone. A concern is a comment (§5.1), not a mark.
- **Flags.** A hunk can be flagged for later: a `Flagged` span over its lines, which does not count as seen. A flag is independent of seen (a hunk can be both). Only flagging again removes it; marking seen or unseen does not. Flags are stored and moved like seen marks, except that they never shrink: a hunk stays flagged while any of its changed lines is flagged, so a flagged hunk that a push splits or edits stays flagged. Finishing the review removes flags with the other marks.
- **When the code changes (I2),** each span is remapped side by side, using the line diff between the file it was made in and the current file, following renames git detects.
  - Only exact matches carry marks. A move Vet only inferred never carries a seen mark.
  - Each reviewed line whose content is unchanged and still in the changeset stays seen. Every other line becomes unseen. One span can therefore become several, or none.
  - If a reviewed line could map to more than one place (for example, repeated identical lines whose surroundings changed), it becomes unseen, rather than leaving the choice to the diff algorithm.
  - A seen region of 20 lines with 3 lines edited in the middle becomes up to two seen spans around 3 unseen lines.
- **Base changes.** If a span's new lines are unchanged but any of its old lines changed, its marks become `SeenAgainstOldBase`. They are told apart from seen marks, count as unseen for completion, and can be confirmed again in one action. This only covers changes inside the reviewed lines; changes elsewhere show up in the impact questions (§6.3), not in the marks. When in doubt, Vet marks a line unseen rather than keeping it seen.
- **Non-text changes.** A change to a file with no text to review (mode, symlink target, submodule commit, binary or oversized content, a pure rename, an added or deleted empty file) is one item. It is acknowledged like a hunk and stored the same way, identified by the file's entries before and after.
- **The diff is byte-level.** Vet marks the byte-level diff. Hiding whitespace, detecting moves or hiding formatting changes only change how it is shown: they never take a change out of the review.

### 5.3 Completion

Completion counts only what the review filter includes. A changeset is complete when every changed line the filter includes is covered by a `Seen` span, and every non-text change it includes is acknowledged. Completion always comes with how many changes the filter excludes, so "complete" never suggests that everything was seen.

## 6. Language features

### 6.1 Language servers (I3)

| Language   | Server                            | Docs in hover for dependencies |
| ---------- | --------------------------------- | ------------------------------ |
| Rust       | rust-analyzer                     | cargo registry sources         |
| TypeScript | typescript-language-server        | JSDoc from `.d.ts`             |
| Svelte     | svelte-language-server            | as TypeScript, plus components |
| Angular    | Angular Language Server           | as TypeScript, plus templates  |
| Python     | Pyright                           | docstrings and typeshed        |
| C#         | roslyn-language-server            | XML docs in NuGet packages     |
| YAML       | yaml-language-server              | schema descriptions (§6.5)     |
| JSON       | vscode-json-languageserver        | schema descriptions (§6.5)     |
| Shell      | bash-language-server              | none                           |
| Dockerfile | dockerfile-language-server-nodejs | instruction docs               |

- The Svelte server loads the project's `svelte.config.js`.
- The Angular server handles only `.html` templates under an `angular.json` (the nearest one up from the file); without one, no server handles the HTML. TypeScript files, inline templates included, use the TypeScript server.
- The Shell server runs ShellCheck when it is installed.
- The Dockerfile server works offline (Docker's own server needs Buildx and calls Docker Scout).

- A file belongs to the language of its extension or, for files like `Dockerfile`, its name. No two languages claim the same one. The backend decides which language a file belongs to and tells the window; the window does not match file names itself. A kind of file several projects share, such as an Angular app's `.html`, is decided by the project's own files.
- The user can set the path of a known server. There are no plugins for other languages.
- **Required:** the server must accept file contents from Vet (text document sync), because Vet sends it the files it asks about. Vet refuses a server without it, with a visible error.
- **Optional, per feature:** `documentSymbol`, `definition`, `references`, `implementation`, `hover`, `callHierarchy` (incoming and outgoing), `workspace/symbol`. Vet always asks for diagnostics (pushed or pulled), because servers publish them without announcing it.
- Vet does not send a request for a feature the server does not support. Instead it answers "not supported by this language's server" (I3). A workspace symbol search names the languages whose servers cannot be searched. A server that supports only some features (only diagnostics, or a YAML server without references) is still used for the rest.

- **Status:** for the old and the new side of the current changeset, the backend reports what each language's server is doing (starting, indexing with its progress, ready, failed, not installed), whenever it changes. Nothing waits for a server for a fixed time.
- **Cancelling:** a request whose answer is no longer needed (a hover the pointer left, a question about an outdated revision) is cancelled at the language server too.

### 6.2 Servers for each snapshot (I1)

- **When servers start.** Language servers, and the dependency restores they need, start for a changeset when the review screen shows it, not when the queue selects it. Selecting changesets in the queue starts nothing, and no background work asks a server while the queue is shown. Showing the review screen starts the servers of the current changeset; while it is shown, a new snapshot, a fetch that moves the current pull request, or a change of trust starts the servers they need. A search the user starts, such as a symbol search, starts what it needs on any screen.
- **The working tree** is served by a server running in the working tree itself. Its answers are about the revision on screen, even when the files on disk have moved on since.
- **Any other snapshot** (a pinned base, an older version, a pull request's base or head) is served by a server in a directory where Vet has materialized the snapshot. The language-server layer only gets the directory and never sees how it was made. Vet materializes it from the snapshot's commit and its content store, and refuses if content is missing (§4.1).
- **Running servers** are kept per snapshot (or working tree), server and analysis policy, and stay running while used.
  - When too many run, servers that are quick to start again, and those unused longest, stop first. Vet never stops a server the current changeset needs for its changed files.
  - Answers outlive their server (I1).
  - A server that fails to start is not tried again by every question. Installing it, changing its path or restarting it tries again at once.
  - Changing a server's settings restarts it. Its cached answers no longer match because the server's fingerprint changed; nothing is deleted.
- **Materialized snapshots are kept between launches** in Vet's data directory: one directory per snapshot and analysis policy, shared by all languages, with a record of each language's dependency restore. Reopening a review with the same base starts its servers without materializing or restoring again. Before reusing a restore, Vet checks it against the current tools and the policy. Old directories are deleted to stay within a disk budget (§9).
- **Dependency restore.** A materialized snapshot needs its dependencies restored for its server to work.
  - By default Vet only downloads them and runs nothing. That is enough for documentation (cargo registry, `.d.ts`, typeshed, NuGet XML docs): no install script, `.pnpmfile.cjs`, build backend or MSBuild step runs.
  - Where the policy allows running project code (§6.5), install scripts and build steps still never run. Only a C# restore (which runs MSBuild) needs that policy.
  - When the npm lock file does not match `package.json`, Vet installs the locked versions anyway, without touching the lock file, and says so as a note, milder than limited analysis. If something also failed, the analysis is reported as limited, with this note among the reasons.
  - A failed step shows the part of the tool's output that explains it. The full output is in the debug log.

### 6.3 Questions Vet answers

- An answer the server gives while it is busy (indexing, or analysing after an edit) is a _partial answer_: it may be incomplete, so Vet never caches it. Diagnostics given that way are marked, and Vet asks again when the server is done.
- Some answers come straight from a server. Others Vet computes from server answers and the diff; these are labeled derived and show what they were computed from (I3).
- When a server points at a file outside the repository (a dependency, the toolchain's sources), Vet shows the line only if the file is of that server's language (§6.1; the Svelte and Angular servers may also point at TypeScript files). Other files are listed without their text.

**From a server:**

- **Hover, go to definition, references, diagnostics:** the server of the side being viewed.
- **Navigation on the old side, next to the new side:** both sides' servers, asked at the same time.
- **Workspace symbols:** `workspace/symbol` on the new side's server.

**Derived:**

- **Which symbols changed:** for each changed file, `documentSymbol` on both sides; a symbol changed if its range overlaps a hunk.
- **Usages of a symbol:** `references` on the new side, in the background, each with the symbol it is in.
- **What a removal breaks:** `references` of each removed symbol on the old side. Each place is then checked on the new side and classed as:
  - **gone**;
  - **modified**;
  - **likely broken:** unchanged, and no longer resolves;
  - **binding changed:** unchanged, but now resolves to something else;
  - **unknown**.
- **Is a diagnostic new:** diagnostics of each changed file on both sides.
  - A new-side diagnostic is old if the old side has one with the same severity, source, code and message (whitespace collapsed) on a line that maps to it. Otherwise it is new.
  - It is _unknown_, never new, while the old side's diagnostics are missing or partial.
  - New-side diagnostics show at once, marked "origin pending".
- **Changed callers and callees of a changed symbol:** incoming and outgoing calls on the new side, followed through changed symbols only. Unchanged direct callers are listed as possibly affected.
- **Is it a test:** by the file's path (`tests/`, `*_test.go`, `*.test.ts`, `*Tests.cs`, …) or an enclosing `tests` module. Labeled "test by file name".

**Text search** (not from a server): over all changes, a range of versions, one commit, or the whole repository without ignored files, on the old side, the new side or both, a line at a time, with Rust's regular expressions.

### 6.4 Reachability (derived, I3)

Reachability says whether new or changed code is reached at all. From each changed or added symbol, Vet follows incoming calls backwards until it hits a configured entry point, within limits on depth and number of symbols. The result is labeled derived:

- `reached` shows the path to the entry point;
- `unreached within bound` shows the limits and the entry points used;
- `unknown` names what servers cannot see (dependency injection, reflection and serialization often hide real calls, worst in C#).

Finding dead code in the whole repository is not a Vet feature. Vet shows it only when a language server reports it as a diagnostic (for example rustc's `dead_code` through rust-analyzer).

### 6.5 Trust

Trust decides whether Vet may run project code while analysing. It is not a security boundary.

- The analysis policy has two parts: whether language servers may run project code (build scripts, proc macros, MSBuild design-time builds), and whether dependency restores may (§6.2). The policy is part of the key of materialized snapshots, restore records and cached answers, so a restore that ran nothing is never reused as one that ran code, and the other way round.
- When trust changes, Vet starts the servers the current changeset needs under the new policy, and stops the running servers that run project code under a policy the current changeset no longer has. A server in use stops as soon as its request ends.
- **Trusted project:** a per-repository setting, off by default. In a trusted project, the working tree, local branches, and pull requests whose head is in the repository itself (not a fork) are analysed with restores and servers that run project code, without asking.
- **Project not trusted:** the working tree and local branches still use servers that run project code, because they are the reviewer's own code, but their restores run nothing. A pull request is limited until trusted: restores run nothing, and only servers that do not run project code start. The analysis of the other languages is reported as limited, with the reason and how to lift it: trusting the pull request or the project.
- **Pull requests from a fork** (the head is in another repository than the one the pull request targets, or Vet never stored the platform's data on it) are limited in a trusted project too; only trusting the pull request lifts it.
- **Trusting a pull request** lifts the limit, for restores and servers alike, for the pull request's current head commit. A new push removes the trust again. A pull request whose head is in a trusted project's own repository is trusted by the project, and a push does not change that. There is no action to remove trust from a pull request; turning off the project's trust removes the project's.
- A pull request's old side runs project code only if its project is trusted, or the pull request is trusted at its current head (the old side is an ancestor of that head). Trusting a pull request covers its head commit only.
- **TypeScript.** The TypeScript server runs a `tsserver.js` through node, and the project may bring its own `typescript`. Where servers may not run project code, Vet uses only the `typescript` it installed itself, or else one installed next to the server (the user's own), never the project's. Without either, the server is reported missing, and Vet offers to install it. The Angular server likewise loads `@angular/language-service`: where servers may not run project code, Vet points it only at its own copies (installed with the server), never at the project's `node_modules`, and without them reports it missing. Vet tells how each server would run for the current changeset.
- **Schemas.** The YAML and JSON servers check files against JSON schemas and would download them (SchemaStore, a `$schema` key, a modeline). Since the repository decides what is downloaded, this follows trust: downloads are allowed only where restores may run code (a trusted project, a trusted pull request). Everywhere else downloading is off, and the servers get a fixed set of schemas that ships with Vet (GitHub Actions workflows, Azure Pipelines, Kubernetes, docker-compose, `package.json`, `tsconfig.json`). Any other schema is refused with a visible diagnostic naming its address, never skipped silently. Where schemas come from is part of the server's fingerprint, and answers obtained with downloads are not reused in later sessions, so answers under different schema rules never mix.

### 6.6 Background analysis

While the review screen shows the current changeset, the backend analyses it in the background and sends each result as soon as it has it. The window does not ask file by file.

- **Changed symbols:** the changed symbols of each changed file, the files the reviewer has open first.
- **Problems:** the diagnostics of each changed file, the files the reviewer has open first. First the new side, marked "origin pending", then the comparison with the old side (§6.3).
- If a file's analysis fails for a reason that may pass (a storage read, a server that is starting, indexing or restarted, a request that failed), Vet tries again a few times before it reports the failure: a server at work is waited for until it is done, not asked again on a timer, and a file waiting for its server does not hold up files of other languages. A file that cannot be analysed for a lasting reason (no server for its language, an untrusted project, a binary or too-large file, a file the changeset does not change, a snapshot that cannot be materialized) is reported at once.
- The analysis follows the current revision. A filter change restarts it for the files the filter now includes. It can be run again on request.

## 7. Platforms

### 7.1 Accounts

- GitHub (including Enterprise hosts) and Azure DevOps. Vet uses a signed-in command-line tool if there is one (`gh auth token`; `az account get-access-token` for Azure DevOps), otherwise a personal access token. There is no browser or device-code login.
- Tokens are kept in the operating system's keychain, never in Vet's storage. In the WSL split, the window keeps them and hands them to the backend. A token is sent only to its own host's API.
- Signing in or out for a host syncs the open pull requests of that host again and reads their checks again.

### 7.2 Platform traits

Each platform is described by a fixed set of properties. Vet decides by these properties, never by the platform's name:

|                                    | GitHub                            | Azure DevOps                                               |
| ---------------------------------- | --------------------------------- | ---------------------------------------------------------- |
| Comments on the whole pull request | a flat conversation               | resolvable threads                                         |
| A conversation feed                | yes                               | no                                                         |
| Posting                            | one review with all comments      | comment by comment                                         |
| Votes                              | comment, approve, request changes | approve, approve with suggestions, wait for author, reject |
| Vote set apart from posting        | no                                | yes                                                        |
| What blocks the merge              | unresolved threads, if required   | unresolved threads, if a policy says so                    |
| Comments belong to a patch set     | no                                | no                                                         |

What follows from them:

- **Review and conversation.** A pull request's comments on files and lines are its review. What is about the pull request as a whole is its conversation: with a conversation feed, the platform's flat list (comments and review summaries with their vote), where your own comments can be edited and are answered with a new comment instead of a reply; without one, the comments on the whole changeset, with replies and resolving.
- **Posting:** drafts are posted as one review with a vote, or one by one. Where the vote is set apart, it is set on its own.
- **Blocking:** whether a thread blocks, and whether a changeset is blocking, follow what blocks the merge on that platform.
- **Platforms that tie comments to a patch set:** Vet still attaches comments to its own snapshots; a platform comment's patch set maps to the snapshot built from that patch set.

### 7.3 Pull requests

- Pull requests are changesets. Vet fetches their heads and targets into its own refs, read-only.
- **The backend keeps the list.** It lists the pull requests again every fetch interval (whatever screen is shown), when the user refreshes and when an account changes, but not just because a list of changesets is shown. Once listed, reading the list never waits for the platform.
- A timed or account-change refresh skips a rate-limited account until its retry time; a refresh the user asks for always asks.
- A repository whose listing fails keeps its last list and reports the error.
- The current pull request is always listed, even when the platform's list leaves it out, from what Vet read when it opened it.
- **Details:** Vet keeps each pull request's details as the platform last gave them (description, reviewers, labels, conversation, state). Reading them never waits for the platform, and refreshes them. After listing, Vet fetches in advance the details of the newest pull requests whose `updatedAt` changed since its copy, at a slow pace, and pauses while their account is rate limited or failing. A pull request that is gone is skipped. If the list changed, this starts again from the top; if not, it continues.

### 7.4 Comment sync

- Comments sync both ways: comments and replies on the platform are pulled into Vet, and drafts in Vet are posted when the user posts them. Seen marks are never synced. Because Vet keeps the commented content (§4.1), comments stay in the right place across rebases where the platform loses them.
- If the user changed a thread's status in Vet since the last sync, that status wins over the platform's.
- An edit to a posted comment is sent at once. If the comment also changed on the platform since the last sync, Vet shows both texts and the user picks one.
- A thread deleted on the platform is deleted in Vet; a draft reply in it is dropped with a notice.
- Drafts are posted on Submit, as one review (GitHub) or one by one (Azure DevOps), never as a pending review on the platform. A comment the platform refused is marked failed with the reason; it is posted again with the next Submit, or can be discarded on its own. A comment being posted cannot be discarded.
- Users can edit and delete their own comments. Deleting the first comment deletes the thread.
- A failed sync is reported as §8 says.

### 7.5 Checks

- Vet reads CI results per commit. While the queue or the review screen shows the current pull request, the backend keeps its checks up to date and sends them: every 30 seconds while a run is unfinished, and shortly after an action on a run, because the platform shows the result with a delay.
- When the account is rate limited, Vet waits until the limit ends, keeping what it last read. A failed checks refresh is reported as §8 says.
- A checks state comes with the runs, jobs and steps behind it, and the log of a finished job, with the failing step marked.
- Vet can re-run a run's failed jobs, re-run a whole run, and cancel an unfinished run. Without a token that may change runs, none of these is possible.

## 8. Background work and reporting

- **Work for the review screen waits for it.** While the queue is shown, Vet keeps up to date only what the queue shows: the lists of changesets, their line counts and pull request details, and for the current changeset its files, the files changed since the last review, checks, comments and pull request details (see also §6.2).
- **Progress.** While a project opens and while background work runs (scanning, diffing, servers starting and indexing, dependency restores), Vet reports which step is running. A changeset's changes are shown as early as possible, not after all work ends.
- **Failures that stand.** A failed sync or checks refresh is reported until one of the same kind succeeds or another changeset becomes current. A failed fetch of the platform remotes (§4.3) is reported until a fetch of them succeeds, whichever changeset is current.
- **Language servers** report their status as §6.1 says.
- **Nothing to review.** A changeset with no changes to show always says why, using only facts the backend has:
  - the working tree has no edits yet;
  - the working tree's edits were undone;
  - the target already contains the branch or pull request, or it has no commits of its own yet;
  - the pull request was merged;
  - the filter excludes every file;
  - the narrowing leaves nothing;
  - the commit changes no file;
  - the push changed nothing.

  Finishing the review stays possible.

## 9. Storage and retention

- **Location:** the backend side's own filesystem (in WSL the Linux home directory, never `/mnt/*`), in the usual data directory of each platform.
- **What is stored:** snapshots and their order, sessions, seen marks, comments, the content store, which projects are trusted, and for each pull request whether it is from a fork and which head commit it is trusted at. Also what the platform last said about recently read pull requests besides their comments (description, reviewers, labels, conversation and state), so revisiting one shows them before Vet asks the platform again. A crash never leaves a record pointing at content that was not written.
- **Upgrades:** a newer Vet keeps all data of an older one. An older Vet does not open the data of a newer one, and never deletes it.
- **Settings and preferences:** everything Vet remembers besides review data is stored with it, except tokens (in the keychain) and the window's size and position (a file of the window).
  - The backend's own settings (language-server cache size, how long finished reviews are kept) apply to the whole app.
  - The window's preferences are stored either for the whole app or for one project.
  - A change is saved at once and sent to every window connected to the backend, which follows it.
- **Retention:** content is kept while an open session, an unresolved or unsynced comment, or a seen mark refers to it. Finished reviews are kept 30 days (configurable). Content nothing refers to is deleted.
- **Language-server cache:** a file of its own, apart from the review data. It holds cached answers (I1) within a size limit (256 MB by default, configurable), dropping the least recently used. It is disposable: deleting it, or a format Vet does not recognise, only makes Vet slower.
- **Materialized snapshots:** one directory per snapshot and analysis policy. They are kept while an open session refers to them, then the least recently used are deleted to stay within a disk budget (20 GB by default, configurable), and after 14 days unused. Vet reports their disk use, and they and the language-server cache can be cleared on request.

## 10. Open question

**Risk ordering:** whether Vet should rank changesets by risk, and what it would use to judge risk.
