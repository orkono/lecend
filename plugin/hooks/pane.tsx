import { atom, read, update } from 'claude-code'
import type { EngineInterface, On } from 'claude-code'

import type { Entry } from '../types'
import { runLecend } from './bin'
import type { LecendResult } from './bin'

export const PANE_ID = 'lecend'
const PANE_TITLE = 'lecend'

// Change the shape tag when Entry's fields change: it declines what an
// earlier version stored, which outlives a reload.
const entries = atom({ plugin: 'lecend', key: 'entries' } as const, [], {
  shape: 'entries-1',
})

function isEntry(value: unknown): value is Entry {
  const entry = value as Partial<Entry> | null
  return (
    typeof entry?.label === 'string' &&
    typeof entry.definition === 'string' &&
    (entry.status === null || typeof entry.status === 'string') &&
    (entry.previously === null || typeof entry.previously === 'string')
  )
}

/** Re-reads `lecend list --json` into the pane's state. */
export async function refreshPane($: EngineInterface): Promise<LecendResult> {
  const listed = await runLecend($, ['list', '--json'])
  if (!listed.ok) {
    return listed
  }
  if (!Array.isArray(listed.json) || !listed.json.every(isEntry)) {
    return { ok: false, error: 'list --json printed an unexpected shape', missing: false }
  }
  const fresh: Entry[] = listed.json
  await update($, entries, () => fresh)

  return listed
}

export async function openPane($: EngineInterface): Promise<void> {
  await $.ui.open({ id: PANE_ID, title: PANE_TITLE })
}

/** Draws the pane: bold label, definition, dimmed status and previous definition. */
export function registerPane(on: On): void {
  on('ui.render', { component: 'Pane', requestId: PANE_ID }, async ($, e) => {
    const { Box, Text } = $.ui.resolve(e)
    const list = await read($, entries)

    return (
      <Box flexDirection="column">
        {list.length === 0 && <Text dimColor>No labels yet.</Text>}
        {list.map(entry => (
          <Box flexDirection="column" marginBottom={1}>
            <Text bold>{entry.label}</Text>
            <Text>{entry.definition}</Text>
            {entry.status !== null && <Text dimColor>→ {entry.status}</Text>}
            {entry.previously !== null && (
              <Text dimColor>previously: {entry.previously}</Text>
            )}
          </Box>
        ))}
      </Box>
    )
  })
}
