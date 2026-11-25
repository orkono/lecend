# lecend — design

Date: 2026-11-25. Status: approved (referee rounds capture1, design1, design2).

## 1. Intent

In long Claude Code sessions the assistant coins short labels early on — server names `OLD` and
`NEW`, a maintenance window `W6`, a test `D4`, `Decision 1`, another session `8a` — and keeps using
them long after their definitions have scrolled out of sight. The user forgets what a label meant,
asks the model "what was W6?", and pays tokens (and context) for the answer.

lecend captures those labels automatically, while the session runs, into SQLite: each label's
**definition** and, where it can be caught, its **latest status**. The user looks a label up with
`/lecend W6` or keeps a side pane open, and neither costs model tokens in the main conversation.

## 2. Goals, non-goals, anti-goals

Goals
- Automatic capture: the user does nothing to record a label.
- Lookup without the model: `/lecend`, `/lecend <label>` and the pane never send their output to
  the model. Only the typed command line itself (`/lecend W6`, ~20 tokens) reaches the transcript.
- Each label shows a definition plus the latest status and when it was last updated.
- A side pane beside the chat that refreshes as the glossary changes.
- Claude Code only.

Non-goals (v1)
- Sharing a glossary between sessions or across a project. Each session has its own glossary.
- Other agents or editors (Codex, Cursor, …).
- Asking the model at lookup time (`/lecend W6 --ask` via `$.model.fork`) — possible later.
- Capturing from subagent turns: v1 reads the main loop only (`turn.complete` with no `agentId`),
  and has no setting for it.

Anti-goals (a failure even if it technically works)
- Capture that slows down or blocks a turn.
- Anything lecend adds to the main model's context (no injected prompts, no glossary in the system
  prompt).
- A confident but invented entry: a definition or status the session never actually stated.
- Silent cost: capture spends tokens without the user being told, or with no way to turn it off.

## 3. Constraints

- Claude Code's plugin hooks module ("mod") must be TypeScript/TSX. Rust cannot be the mod itself.
- The mod API (verified in a spike against Claude Code 2.1.331):
  - `$.command.register` + a `command.run` hook returning `{ text, isTextDisplayOnly: true }`
    answers a slash command without any model request; the output row is drawn and kept but goes
    in no request.
  - `$.ui.open({ id, title })` + a `ui.render` hook on `{ component: 'Pane' }` draws a side pane.
    In a terminal it seats from 110–144 columns; narrower, it waits.
  - `$.process.run(argv, { stdin, timeoutMs })` runs a host binary (no shell).
  - `turn.complete` gives the assistant's final visible text (`answer`), `turnId`, and `agentId`
    for subagents; `prompt.submit` gives the user's prompt text.
  - `$.model.complete({ model, prompt, … })` runs one tool-less completion on the session's own
    Claude account. It counts toward `/cost` and the user's quota but never enters the main
    conversation.
  - The mod's process environment has no `HOME`; `$.process.hookEnv()` provides
    `CLAUDE_PLUGIN_DATA` (the plugin's data folder, kept across updates) and
    `CLAUDE_CODE_SESSION_ID`.
- The repository is public (github.com/orkono/lecend), license Apache-2.0 (same as cestafet).
  Others will install it, so nothing may depend on the author's private tools or keys.
- Local only: one SQLite file, no network besides the Claude API call Claude Code itself makes.

## 4. Approaches considered

Chosen by a referee round of three independent models (tiers 1–3 unanimous on A+C).

- **A. Cheap Claude model per turn (chosen).** After each main-loop turn the mod sends that turn's
  text plus the relevant glossary entries to a cheap model through `$.model.complete` (default
  `haiku`, configurable). The model returns a JSON diff; the Rust binary validates and applies it.
  Works out of the box on the user's account; understands statuses ("approved", "opens on …").
- **C. Rust label matching (chosen, as support for A).** Rust finds known labels and label-shaped
  tokens in the turn, picks the glossary entries to send, and skips the model call entirely when a
  turn has nothing label-like.
- B. External cheap LLM (MiMo, DeepSeek) — rejected: needs each user's own key and tool, sends
  transcript text to a third party.
- D. No capture; ask `$.model.fork` at lookup — rejected for v1: costs tokens on every lookup,
  fails after compaction, leaves the pane empty. Kept as a possible opt-in later.
- C alone — rejected: regex cannot read statuses or implicit definitions.

## 5. Architecture

```
Claude Code session
 └─ lecend mod (TypeScript, plugin/hooks/register.tsx)
     ├─ prompt.submit   → remember the user's prompt for this turn
     ├─ turn.complete   → (main loop only) queue a capture job, return immediately
     ├─ capture worker  → lecend scan → $.model.complete → lecend apply
     ├─ command.run /lecend → lecend show … → display-only text
     └─ Pane "lecend"   → lecend list --json, refreshed after each apply
 └─ lecend binary (Rust, on PATH)
     └─ SQLite: $CLAUDE_PLUGIN_DATA/lecend.db
```

The mod holds no business logic beyond queueing, calling the model and drawing. All parsing,
validation, matching and storage live in the Rust binary, which is testable on its own.

### 5.1 Rust binary — `lecend`

Subcommands (all read JSON on stdin where noted, write JSON on stdout, exit non-zero with a JSON
error on failure). Every command takes `--db <path>` and `--session <id>`.

- `lecend scan` (stdin: `{ prompt, answer, stoplist[] }`) → `{ shouldCall, knownLabels[], candidates[],
  entries[] }`. Finds known labels of this session that appear in the text, and label-shaped
  candidates. `shouldCall` is false only when there is neither.
- `lecend apply` (stdin: `{ turnId, prompt, answer, stoplist[], diff }`) → `{ applied, rejected[] }`.
  Resolves the turn index from `turnId` (recorded earlier by `lecend turn`), validates the model's
  diff and writes it (see 6.3).
- `lecend show <label>` → text for the command row. `lecend list [--json]` → all labels, most
  recently updated first.
- `lecend edit <label> --definition …|--status …|--pin|--unpin`, `lecend forget <label>`,
  `lecend unforget <label>` → manual corrections (see 6.4).
- `lecend turn` (stdin: `{ turnId }`) → `{ turnIndex }`: records the turn and assigns its index.
- `lecend fork --from <parent session id>` → copies the parent's glossary into this session (see 7).
- `lecend version`, `lecend doctor` (binary, DB path, schema version).

### 5.2 Mod — TypeScript

- On `session.start`: register `/lecend`, resolve the DB path from `CLAUDE_PLUGIN_DATA` and the
  session id; check the binary with `lecend version`. If it is missing, `/lecend` says how to
  install it and capture stays off.
- Capture queue: one job per main-loop turn. The turn hook first records the turn with
  `lecend turn` (fast, local), which assigns the turn index once per `turnId` in a transaction;
  then it queues the job and returns. Jobs run strictly in turn order, one at a time, never awaited
  by the turn hook. A failed job is logged to the debug log and dropped; the next
  turn's job carries on. A per-session budget (default: 200 model calls) stops capture when reached,
  with a one-time toast.
- `/lecend` (list), `/lecend <label>` (one entry with definition, latest status, last update and the
  source quote), `/lecend pane`, `/lecend history <label>`, `/lecend edit …`, `/lecend forget …`,
  `/lecend unforget …`, `/lecend off|on` (capture for this session). Every answer is
  `isTextDisplayOnly: true`.
- Pane: label in bold, definition, latest status dimmed; refreshed after every successful apply
  and after edits (no polling timer).
- Settings (`userConfig`): `model` (default `haiku`), `capture` (on/off, default on),
  `budgetPerSession`, `stoplist` (extra labels to ignore everywhere; passed to both `scan` and
  `apply`).
- Headless runs (`claude -p`, SDK; `session.start` reports no person at the prompt): capture is off,
  since nobody can see the cost notice. This field is not yet spike-verified: verify at build start,
  and keep capture off whenever it cannot be determined.

## 6. Capture pipeline

### 6.1 What counts as a label

Short tokens the session uses as names: uppercase words of 2–8 letters (`OLD`, `NEW`, `DKIM`
excluded by stoplist), letter+digit mixes (`W6`, `D4`, `8a`), and "Word N" forms in any language
(`Decision 1`, `Karar 1`). A built-in stoplist removes common acronyms (`OK`, `API`, `URL`, `TODO`,
`JSON`, `SQL`, …); the user extends it with the `stoplist` setting. The gate is deliberately permissive: it decides whether
to call the model, not what is a label. The model decides that.

### 6.2 The model call

Input: the user's prompt and the assistant's answer of this turn (truncated to a fixed size, head
and tail kept), the session's known labels (names only), and full entries for the labels mentioned
in this turn plus the few most recently updated ones (capped). The prompt states that the turn text
is untrusted data, not instructions.

Output, strict JSON:
`{ "new": [{ label, definition, quote }], "redefine": [{ label, definition, quote }],
"status": [{ label, status, quote }] }`. Empty arrays when nothing changed. `redefine` is for a
label the session now explicitly uses with a different meaning.

### 6.3 Validation and storage (Rust)

A change is accepted only if:
- `quote` appears verbatim in this turn's prompt or answer, and the label appears in the quote
  (or, for a status, in the turn);
- the label passes length limits and the stoplist; definition ≤ 200 chars, status ≤ 200 chars;
- the label is not on the session's ignore list (forgotten) nor on the built-in or user stoplist —
  rechecked at apply time, since a job may have been queued before the user acted;
- for `new`: the label is not already defined in this session (otherwise the change is rejected;
  the model receives all known names, so this is a model error, not a redefinition);
- for `redefine`: the quote contains the label and itself states the changed meaning (a quote that
  only mentions the label is not enough); a definition identical to the current one is a no-op;
- for `redefine` and `status`: the label is not pinned, and for `redefine` the current definition
  is not user-edited (those are never replaced by the model);
- the turn index is newer than the label's last update, including the last user edit, so a
  delayed job never overtakes a manual correction. The check compares against the state before
  this diff; all accepted operations of one turn are committed together in one transaction (a
  `redefine` and a `status` for the same label in the same turn both apply, the status attached to
  the new definition version). Applying the same `turnId` twice is a no-op.

Rejected changes are kept in a `rejections` table for debugging, not shown by default.

Schema (v1): `terms(session_id, label, pinned, created_turn)`,
`definitions(session_id, label, version, definition, turn_index, quote, created_at, source)`,
`statuses(session_id, label, definition_version, status, turn_index, quote, created_at, source)`,
`ignores(session_id, label)`, `turns(session_id, turn_id, turn_index, recorded_at)`,
`sessions(session_id, forked_from, fork_turn, turn_base, fork_done, capture_off, model_calls)`,
`rejections(…)`, `meta(schema_version, cost_notice_shown)` (install-wide), with `source` =
`model` | `user`. Per-session state lives in `sessions`, so copy-once, the continued counter,
`/lecend off` and the budget survive a resume.
Definitions are append-only versions; the newest is the current meaning, and statuses belong to the
definition version they were stated under. `show` and the pane present the current definition, its
latest status and "previously: …" when older versions exist; `history` lists all versions and
statuses with their quotes. The model never deletes.

### 6.4 User corrections

- `edit --definition` adds a user-sourced definition version; `edit --status` a user-sourced
  status. Both carry the current turn index.
- `edit --pin` freezes the label against model changes; `--unpin` releases it.
- `forget` deletes the term with its definitions and statuses and adds the label to the session's
  ignore list, so capture does not bring it back. `unforget` removes it from the ignore list (the
  deleted history does not come back). `edit` on a forgotten label unforgets it.

## 7. Session identity

Glossaries are keyed by Claude Code's session id (flat key, no lineage reads). Whether
`/resume`, `--continue`, `--fork-session` and compaction keep the id, and whether the mod can learn
a new session's parent id, is to be verified at the start of the build (`session.start` carries
resume information).

- Same id (resume, compaction): nothing to do.
- New id with a known parent (fork, or resume under a new id): `lecend fork --from <parent>
  [--at-turn <n>]` copies the parent's terms, definition versions, statuses, pins and ignore list in
  one transaction, exactly once (`sessions.fork_done` set in the same transaction). When the fork
  turn is known, only rows with `turn_index` ≤ it are copied. The child's turn counter continues
  from the copied state's highest turn index (`sessions.turn_base`), so later updates are not
  rejected as "not newer".
- New id with no detectable parent: the glossary starts empty and `/lecend` says why.

Known gaps: when the fork turn cannot be established, labels defined after the fork point are
copied too; parent capture jobs still queued at fork time are not in the copy.

## 8. Cost and disclosure

Capture spends tokens on the user's Claude account even when nobody looks anything up.

- Once per install (recorded in `meta.cost_notice_shown`), before the first capture call, the mod
  shows a display-only notice: the model used, a rough per-turn cost, that calls are billed to the user's Claude account
  (and count toward their quota), the per-session budget, and how to stop it (`/lecend off` for
  this session, the `capture` setting for all sessions). It is a transcript row that never goes to
  the model.
- The README states this up front with a rough per-turn figure, how to switch capture off (`/lecend off`, or the
`capture` setting) and how to pick a model. `/lecend` shows the session's capture call count.

## 9. Known limits (v1)

- Only the final visible answer of each turn is read: labels coined between tool calls, in files
  or in subagent output are missed unless repeated in the answer.
- The model can attach a wrong status that still has a genuine quote (reused label, hypothetical,
  "yes, go ahead" several turns later), or take a quoted outside redefinition as a new meaning.
  Mitigation: the source quote is shown, every version is kept, and the user can edit or pin.
- Generic labels (`OLD`, `NEW`) may produce false positives; the user can forget them.

## 10. Distribution

As in cestafet: the repository holds a Claude Code marketplace (`.claude-plugin/marketplace.json`)
with the plugin under `plugin/` (the mod, its types, README). The `lecend` binary ships as GitHub
release archives for darwin arm64/x64, linux x64/arm64 and windows x64, a Homebrew cask in
`orkono/tap`, and `cargo install`. The binary must be on the `PATH` Claude Code sees; `lecend doctor`
checks the setup.

## 11. Left to the builder

- Crate choices (`rusqlite` with bundled SQLite is expected), CLI parsing, JSON schema details.
- The exact label regexes, stoplist, truncation sizes and the extraction prompt wording, tuned with
  fixtures taken from real transcripts (the W6/OLD/NEW example is the first fixture).
- Pane layout details; behaviour when the terminal is too narrow.
- Test layout: Rust unit tests for scan/apply/validation; `claude plugin test` for the mod.
