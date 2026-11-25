import { expect, test } from 'claude-code/testing'
import type { On } from 'claude-code'
import type { Engine } from 'claude-code/testing'

import { parseArgs } from './commands'

const INSTALL =
  'lecend binary not found on PATH. Install: brew install --cask orkono/tap/lecend  (or: cargo install lecend). Then restart the session.'

const ENTRIES = [
  {
    label: 'W6',
    definition: 'the sixth wave of the migration',
    status: 'done',
    previously: 'the sixth week',
    updatedTurn: 4,
    pinned: false,
  },
]

type Reply = { exitCode?: number; stdout: string }

/** Answers `lecend` runs by subcommand; records every argv. */
function fakeBinary(on: On, replies: Record<string, Reply>): string[][] {
  const calls: string[][] = []
  on('process.hookEnv', () => ({
    value: { CLAUDE_PLUGIN_DATA: '/data', CLAUDE_CODE_SESSION_ID: 'sess-1' },
  }))
  on('process.run', (_, e) => {
    calls.push([...e.argv])
    const reply = replies[e.argv[1] ?? ''] ?? { stdout: '{"ok":true}' }

    return {
      value: {
        exitCode: reply.exitCode ?? 0,
        stdout: reply.stdout,
        stderr: '',
        isStdoutTruncated: false,
        isStderrTruncated: false,
      },
    }
  })

  return calls
}

/** Runs `/lecend <args>` as typed in the composer. */
function lecend($: Engine, args: string) {
  return $.command.run({
    command: 'lecend',
    args,
    origin: { kind: 'composer' },
    presentation: { isFullscreen: false, columns: 80 },
  })
}

const STATE =
  '{"captureOff":false,"modelCalls":7,"forkedFrom":null,"forkTurn":null,"forkDone":false,"labels":1,"turns":9}'

const BASE: Record<string, Reply> = {
  version: { stdout: '{"version":"0.1.0"}' },
  state: { stdout: STATE },
  show: { stdout: 'W6  the sixth wave of the migration\n    status: done' },
}

test('parseArgs reads every syntax', () => {
  const table: [string, unknown][] = [
    ['', { kind: 'list' }],
    ['   ', { kind: 'list' }],
    ['W6', { kind: 'show', label: 'W6' }],
    ['Karar 1', { kind: 'show', label: 'Karar 1' }],
    ['pane', { kind: 'pane' }],
    ['history W6', { kind: 'history', label: 'W6' }],
    ['history Karar 1', { kind: 'history', label: 'Karar 1' }],
    [
      'edit Karar 1 = karar tanımı',
      { kind: 'edit', label: 'Karar 1', field: 'definition', text: 'karar tanımı' },
    ],
    [
      'status W6 = done = shipped',
      { kind: 'edit', label: 'W6', field: 'status', text: 'done = shipped' },
    ],
    ['pin W6', { kind: 'pin', label: 'W6' }],
    ['unpin W6', { kind: 'unpin', label: 'W6' }],
    ['forget Karar 1', { kind: 'forget', label: 'Karar 1' }],
    ['unforget W6', { kind: 'unforget', label: 'W6' }],
    ['off', { kind: 'off' }],
    ['on', { kind: 'on' }],
    ['show pane', { kind: 'show', label: 'pane' }],
    ['show Karar 1', { kind: 'show', label: 'Karar 1' }],
    ['OFF', { kind: 'show', label: 'OFF' }],
    ['edit W6', { kind: 'usage' }],
    ['edit W6 = ', { kind: 'usage' }],
    ['pin', { kind: 'usage' }],
    ['pane W6', { kind: 'usage' }],
  ]
  for (const [args, command] of table) {
    expect(parseArgs(args)).toEqual(command)
  }
})

test('list footer shows capture state and calls', async ($, on) => {
  const calls = fakeBinary(on, { ...BASE, list: { stdout: 'W6  the sixth wave — done' } })

  const answer = await lecend($, '')

  expect(answer.text).toBe(
    'W6  the sixth wave — done\ncapture: on · 7 model calls this session',
  )
  expect(calls).toContainEqual(['lecend', 'list', '--db', '/data/lecend.db', '--session', 'sess-1'])
})

test('/lecend W6 answer is display-only', async ($, on) => {
  const calls = fakeBinary(on, BASE)

  const answer = await lecend($, 'W6')

  expect(answer.text).toBe('W6  the sixth wave of the migration\n    status: done')
  expect(answer.isTextDisplayOnly).toBe(true)
  expect(answer.context).toBeUndefined()
  expect(calls).toContainEqual([
    'lecend', 'show', '--db', '/data/lecend.db', '--session', 'sess-1', '--', 'W6',
  ])
})

test('edit runs the binary then answers the new show text', async ($, on) => {
  on('ui.open', () => ({ value: { isPlaced: true } }))
  const calls = fakeBinary(on, { ...BASE, list: { stdout: '[]' } })

  const answer = await lecend($, 'edit W6 = a new meaning')

  expect(answer.text).toBe('W6  the sixth wave of the migration\n    status: done')
  expect(answer.isTextDisplayOnly).toBe(true)
  expect(calls).toContainEqual([
    'lecend', 'edit', '--db', '/data/lecend.db', '--session', 'sess-1',
    '--definition', 'a new meaning', '--', 'W6',
  ])
  expect(calls).toContainEqual([
    'lecend', 'list', '--db', '/data/lecend.db', '--session', 'sess-1', '--json',
  ])
})

test('forget answers Forgotten and off switches capture', async ($, on) => {
  const calls = fakeBinary(on, {
    ...BASE,
    list: { stdout: '[]' },
    capture: { stdout: '{"captureOff":true}' },
  })

  const forgot = await lecend($, 'forget Karar 1')
  const off = await lecend($, 'off')

  expect(forgot.text).toBe('Forgotten Karar 1.')
  expect(off.text).toBe('lecend capture is off for this session.')
  expect(calls).toContainEqual([
    'lecend', 'capture', '--db', '/data/lecend.db', '--session', 'sess-1', '--off',
  ])
})

test('failed pane refresh after edit is logged', async ($, on) => {
  const logged: { text: string; to: unknown }[] = []
  on('ui.log', (_, e) => {
    logged.push({ text: e.text, to: e.to })

    return { value: undefined }
  })
  fakeBinary(on, { ...BASE, list: { exitCode: 1, stdout: '{"error":"db locked"}' } })

  const answer = await lecend($, 'pin W6')

  expect(answer.text).toBe('W6  the sixth wave of the migration\n    status: done')
  expect(logged).toEqual([{ text: 'lecend: pane not refreshed: db locked', to: 'debug' }])
})

test('a binary error is shown as lecend: <error>', async ($, on) => {
  fakeBinary(on, {
    ...BASE,
    edit: { exitCode: 1, stdout: '{"error":"no label \\"Q\\""}' },
  })

  const answer = await lecend($, 'pin Q')

  expect(answer.text).toBe('lecend: no label "Q"')
  expect(answer.isTextDisplayOnly).toBe(true)
})

test('missing binary gives install text', async ($, on) => {
  on('process.hookEnv', () => ({ value: { CLAUDE_PLUGIN_DATA: '/data' } }))
  on('process.run', () => {
    throw new Error('spawn lecend ENOENT')
  })

  for (const args of ['', 'W6', 'pane', 'off']) {
    const answer = await lecend($, args)
    expect(answer.text).toBe(INSTALL)
    expect(answer.isTextDisplayOnly).toBe(true)
  }
})

test('a binary missing at session.start is not run again', async ($, on) => {
  on('command.register', () => ({ value: { command: 'lecend' } }))
  on('session.start', (_, e) => ({ cwd: e.cwd }))
  const calls = fakeBinary(on, { ...BASE, version: { exitCode: 127, stdout: '' } })

  await $.session.start({ cwd: '/', surface: 'terminal', isInteractive: true })
  const answer = await lecend($, 'W6')

  expect(answer.text).toBe(INSTALL)
  expect(calls).toEqual([['lecend', 'version']])
})

const PANE = {
  plugin: 'lecend',
  component: 'Pane',
  requestId: 'lecend',
  props: {
    title: 'lecend',
    isFocused: false,
    bodyColumns: 40,
    placement: 'dock',
    scroll: { offset: 0, bodyRows: 20 },
    view: {},
  },
} as const

test('pane renders entries', async ($, on) => {
  on('ui.open', () => ({ value: { isPlaced: true } }))
  fakeBinary(on, { ...BASE, list: { stdout: JSON.stringify(ENTRIES) } })

  await lecend($, 'pane')

  for (const surface of ['terminal', 'desktop'] as const) {
    const ui = await $.ui.mount({ ...PANE, surface })
    expect((await ui.find({ type: 'Text', text: /^W6$/ }))?.text).toBe('W6')
    expect(await ui.find({ type: 'Text', text: /sixth wave/ })).toBeDefined()
    expect((await ui.find({ type: 'Text', text: /→/ }))?.text).toBe('→ done')
    expect((await ui.find({ type: 'Text', text: /previously/ }))?.text).toBe(
      'previously: the sixth week',
    )
    await ui.unmount()
  }
})

test('pane not drawn yet says the engine\'s reason', async ($, on) => {
  on('ui.open', () => ({ value: { isPlaced: false, reason: 'no attached surface places panes' } }))
  fakeBinary(on, { ...BASE, list: { stdout: JSON.stringify(ENTRIES) } })

  const answer = await lecend($, 'pane')

  expect(answer.text).toBe('lecend pane is open but not drawn yet: no attached surface places panes')
  expect(answer.isTextDisplayOnly).toBe(true)
})

test('pane refused by the engine answers its reason', async ($, on) => {
  on('ui.open', () => ({ deny: 'pane held by another plugin' }))
  fakeBinary(on, { ...BASE, list: { stdout: JSON.stringify(ENTRIES) } })

  const answer = await lecend($, 'pane')

  expect(answer.text).toMatch(/^lecend: .*pane held by another plugin$/)
  expect(answer.text).not.toMatch(/^lecend: lecend:/)
  expect(answer.isTextDisplayOnly).toBe(true)
})
