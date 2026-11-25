# Session identity — findings (Task 1, 2026-11-25)

Measured with a throwaway logging mod (`session.start`: `$.session.id()`, `CLAUDE_CODE_SESSION_ID`,
`isInteractive`) on Claude Code 2.1.331, headless runs (`claude -p --plugin-dir …`), plus the
transcripts those runs wrote. Rows marked "docs" come from the mod API's type documentation, not a run.

| Case | Session id | Parent discoverable | Fork turn discoverable |
|---|---|---|---|
| Fresh start | new | n/a | n/a |
| `--resume <id>` | same | n/a | n/a |
| `--continue` | same | n/a | n/a |
| `/compact` (via `--resume <id> "/compact"`) | same | n/a | n/a |
| `--resume <id> --fork-session` | **new** | yes, see below | yes, see below |
| `/clear` (docs) | `session.end` with `reason: 'clear'`, no `session.start` after | n/a | n/a |

`$.session.id()` and `CLAUDE_CODE_SESSION_ID` always agreed.

## Fork detection

The forked session's transcript rewrites every entry's `sessionId` to the new id and carries no
parent field. But the copied messages keep their original `uuid`s: the fork's first user and
assistant messages have exactly the uuids of the parent's. The parent is therefore the other
transcript in the same project folder that contains the fork's first user-message uuid.

The transcript path is available to the mod through `$.session.info().transcriptPath`
(`classic.SessionStart` did not reach a mod hook in a `-p` run, so it is not relied on).

Rule for Task 11:
- On the first main-loop `turn.complete` of a session that has no `sessions` row yet (lazy, so the
  forked transcript is surely written), run `lecend lineage --transcript <path>`. The binary reads the
  transcript's user-message uuids, scans sibling `*.jsonl` files in the same folder for the first one,
  and returns `{ "parent": <id or null>, "sharedUserTurns": N }`.
- `parent` found and it has a glossary → `lecend fork --from <parent> --at-turn N`, where N is the
  parent's turn index of the last shared user turn (≈ number of shared user prompts; an
  approximation, since the parent's index counts completed main-loop turns).
- No transcript, no shared uuids, or a parent with no glossary → nothing to copy; the session is
  treated as fresh. A fork whose parent transcript is gone cannot be told apart from a fresh start
  (copied entries look like originals), so v1 never calls `fork --none` from the mod; the
  subcommand stays for completeness and tests.

## isInteractive

- `claude -p`: `false` (measured, every run).
- Terminal REPL: `true` (docs; this very session draws in a terminal).
- Agent SDK: `false` (docs; not run locally). Capture is fail-closed anyway.

## Cost notice

`$.session.append({ message: { type: "system", content: [{ type: "text", text }] } })` writes a
notice "the model never reads", stored at once (docs, `SessionAppendArgs`). Use it for the cost
notice. The end-to-end check (Task 13) confirms the model cannot see it.
