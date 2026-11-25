import type { EngineInterface } from 'claude-code'

import { INSTALL_TEXT, hasBinary, runLecend } from './bin'
import type { LecendResult } from './bin'
import { openPane, refreshPane } from './pane'

export type Command =
  | { kind: 'list' }
  | { kind: 'show'; label: string }
  | { kind: 'pane' }
  | { kind: 'history'; label: string }
  | { kind: 'edit'; label: string; field: 'definition' | 'status'; text: string }
  | { kind: 'pin' | 'unpin' | 'forget' | 'unforget'; label: string }
  | { kind: 'off' | 'on' }
  | { kind: 'usage' }

type Change = Extract<Command, { kind: 'edit' | 'pin' | 'unpin' | 'forget' | 'unforget' }>

export const USAGE = [
  'Usage:',
  '  /lecend                      list labels',
  '  /lecend <label>              show a label (or: /lecend show <label>)',
  '  /lecend pane                 open the side pane',
  '  /lecend history <label>',
  '  /lecend edit <label> = <definition>',
  '  /lecend status <label> = <status>',
  '  /lecend pin|unpin|forget|unforget <label>',
  '  /lecend off|on               switch capture for this session',
].join('\n')

const LABELLED = ['show', 'history', 'pin', 'unpin', 'forget', 'unforget'] as const
const BARE = ['pane', 'off', 'on'] as const

function splitAssignment(rest: string): { label: string; text: string } | undefined {
  const at = rest.indexOf(' = ')
  if (at < 0) {
    return undefined
  }
  const label = rest.slice(0, at).trim()
  const text = rest.slice(at + 3).trim()

  return label === '' || text === '' ? undefined : { label, text }
}

/**
 * Reads what follows `/lecend`. Reserved first words match exactly (lower
 * case), so `/lecend OFF` looks up a label; `/lecend show <word>` looks up a
 * label named like a reserved word.
 */
export function parseArgs(args: string): Command {
  const all = args.trim()
  if (all === '') {
    return { kind: 'list' }
  }
  const [first = ''] = all.split(/\s/, 1)
  const rest = all.slice(first.length).trim()
  const bare = BARE.find(word => word === first)
  if (bare !== undefined) {
    return rest === '' ? { kind: bare } : { kind: 'usage' }
  }
  const labelled = LABELLED.find(word => word === first)
  if (labelled !== undefined) {
    return rest === '' ? { kind: 'usage' } : { kind: labelled, label: rest }
  }
  if (first === 'edit' || first === 'status') {
    const parts = splitAssignment(rest)
    const field = first === 'edit' ? 'definition' : 'status'

    return parts === undefined ? { kind: 'usage' } : { kind: 'edit', field, ...parts }
  }

  return { kind: 'show', label: all }
}

/** The binary's text, the install text, or `lecend: <error>`. */
function textOf(result: LecendResult): string {
  if (result.ok) {
    return result.text
  }

  return result.missing ? INSTALL_TEXT : `lecend: ${result.error}`
}

async function listAnswer($: EngineInterface): Promise<string> {
  const list = await runLecend($, ['list'])
  if (!list.ok) {
    return textOf(list)
  }
  const state = await runLecend($, ['state'])
  if (!state.ok) {
    return textOf(state)
  }
  const capture = state.json?.captureOff === true ? 'off' : 'on'
  const calls = Number(state.json?.modelCalls ?? 0)

  return `${list.text}\ncapture: ${capture} · ${calls} model calls this session`
}

function changeArgs(change: Change): string[] {
  switch (change.kind) {
    case 'edit':
      return ['edit', `--${change.field}`, change.text, '--', change.label]
    case 'pin':
    case 'unpin':
      return ['edit', `--${change.kind}`, '--', change.label]
    default:
      return [change.kind, '--', change.label]
  }
}

/** Runs a change, refreshes the pane, then answers the label's new state. */
async function changeAnswer($: EngineInterface, change: Change): Promise<string> {
  const changed = await runLecend($, changeArgs(change))
  if (!changed.ok) {
    return textOf(changed)
  }
  await refreshPane($)
  if (change.kind === 'forget') {
    return `Forgotten ${change.label}.`
  }
  if (change.kind === 'unforget') {
    return `Unforgotten ${change.label}.`
  }

  return textOf(await runLecend($, ['show', '--', change.label]))
}

async function captureAnswer($: EngineInterface, kind: 'off' | 'on'): Promise<string> {
  const ran = await runLecend($, ['capture', `--${kind}`])

  return ran.ok ? `lecend capture is ${kind} for this session.` : textOf(ran)
}

async function paneAnswer($: EngineInterface): Promise<string> {
  const refreshed = await refreshPane($)
  if (!refreshed.ok) {
    return textOf(refreshed)
  }
  await openPane($)

  return 'lecend pane opened.'
}

/** The display-only text that answers one `/lecend` command. */
export async function answer($: EngineInterface, command: Command): Promise<string> {
  if (!(await hasBinary($))) {
    return INSTALL_TEXT
  }
  switch (command.kind) {
    case 'list':
      return listAnswer($)
    case 'show':
    case 'history':
      return textOf(await runLecend($, [command.kind, '--', command.label]))
    case 'pane':
      return paneAnswer($)
    case 'off':
    case 'on':
      return captureAnswer($, command.kind)
    case 'usage':
      return USAGE
    default:
      return changeAnswer($, command)
  }
}
