import type { EngineInterface } from 'claude-code'

import { currentSessionId, runLecend } from './bin'
import { refreshPane } from './pane'

// Every call here is bounded to 2 s so the first turn is never held long:
// state, lineage, parent state and fork add up to about 8 s in the worst case,
// once per session. `fork` is one IMMEDIATE transaction in the binary, so a
// timeout cannot leave a partial copy. The pane refresh is not awaited.
const LINEAGE_TIMEOUT_MS = 2_000

// Session ids already checked in this module's life; a session is checked once.
const checked = new Set<string>()

type Lineage = { parent: string; sharedUserTurns: number }

function debug($: EngineInterface, text: string): void {
  $.ui.log(`lecend: ${text}`, { to: 'debug' })
}

/** Runs `lecend lineage` on a transcript; it takes no `--db` or `--session`. */
async function findParent($: EngineInterface, transcript: string): Promise<Lineage | null> {
  const ran = await $.process.run(['lecend', 'lineage', '--transcript', transcript], {
    timeoutMs: LINEAGE_TIMEOUT_MS,
  })
  if (ran.exitCode !== 0) throw new Error(`exited with code ${ran.exitCode}: ${ran.stdout.trim()}`)
  const found = JSON.parse(ran.stdout) as { parent?: unknown; sharedUserTurns?: unknown }
  if (found.parent === null) return null
  if (typeof found.parent !== 'string' || typeof found.sharedUserTurns !== 'number') {
    throw new Error('printed an unexpected shape')
  }

  return { parent: found.parent, sharedUserTurns: found.sharedUserTurns }
}

async function stateOf($: EngineInterface, session?: string): Promise<Record<string, unknown>> {
  const ran = await runLecend($, ['state'], undefined, LINEAGE_TIMEOUT_MS, { session })
  if (!ran.ok) throw new Error(`state: ${ran.error}`)
  if (typeof ran.json !== 'object' || ran.json === null) throw new Error('state: printed no JSON object')

  return ran.json
}

async function copyFromParent($: EngineInterface, { parent, sharedUserTurns }: Lineage): Promise<void> {
  const parentState = await stateOf($, parent)
  if (typeof parentState.labels !== 'number' || parentState.labels === 0) return
  const forked = await runLecend(
    $,
    ['fork', '--from', parent, '--at-turn', String(sharedUserTurns)],
    undefined,
    LINEAGE_TIMEOUT_MS,
  )
  if (!forked.ok) throw new Error(`fork: ${forked.error}`)
  void refreshPane($).then(
    refreshed => {
      if (!refreshed.ok) debug($, `pane not refreshed: ${refreshed.error}`)
    },
    err => debug($, `pane not refreshed: ${(err as Error)?.message ?? err}`),
  )
}

/**
 * On a session's first captured turn, before `lecend turn` records it: when the
 * session is a `--fork-session` of one that has a glossary, copies that glossary
 * (docs/notes/session-identity.md). Any failure is logged; capture goes on.
 */
export async function carryGlossary($: EngineInterface): Promise<void> {
  let session = ''
  try {
    session = await currentSessionId($)
    if (checked.has(session)) return
    const { transcriptPath } = await $.session.info()
    if (transcriptPath === '') return
    checked.add(session)
    const state = await stateOf($)
    if (state.turns !== 0 || state.forkDone !== false) return
    const parent = await findParent($, transcriptPath)
    if (parent !== null) await copyFromParent($, parent)
  } catch (err) {
    debug($, `lineage check for session ${session} failed: ${(err as Error)?.message ?? err}`)
  }
}
