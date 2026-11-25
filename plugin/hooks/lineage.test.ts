import { expect, mock, test } from 'claude-code/testing'

import { fakeWorld, start, subs, turn } from './capture.fakes'
import type { World } from './capture.fakes'

// The lineage check runs once per session id for the life of the module, so
// every test uses a session id of its own.
function forkedWorld(on: Parameters<typeof fakeWorld>[0], session: string): World {
  const world = fakeWorld(on)
  world.session = session
  world.transcript = `/projects/p/${session}.jsonl`
  world.lineage = { parent: 'parent-1', sharedUserTurns: 3 }
  world.parentLabels = 4

  return world
}

const forks = (world: World) => world.events.filter(event => event.startsWith('fork:'))

test('fork with parent glossary copies before first turn is recorded', async ($, on) => {
  const clock = mock.clock(on)
  const world = forkedWorld(on, 'fork-a')

  await start($)
  await turn($, clock, 't-1', 'W6 is a window', 'Define W6')

  expect(forks(world)).toEqual(['fork:parent-1@3'])
  expect(world.events.indexOf('fork:parent-1@3')).toBeLessThan(world.events.indexOf('bin:turn'))
  const lineage = world.runs.find(run => run.sub === 'lineage')
  expect(lineage?.argv).toEqual(['lecend', 'lineage', '--transcript', '/projects/p/fork-a.jsonl'])
  expect(lineage?.timeoutMs).toBe(2000)
  const parentState = world.runs.find(run => run.sub === 'state' && run.argv.includes('parent-1'))
  expect(parentState).toBeDefined()
  expect(subs(world)).toContain('list')
})

test('carry-over calls use the 2 s timeout', async ($, on) => {
  const clock = mock.clock(on)
  const world = forkedWorld(on, 'timeouts-a')

  await start($)
  await turn($, clock, 't-1', 'W6 is a window')

  const beforeTurn = world.runs.slice(0, world.runs.findIndex(run => run.sub === 'turn'))
  const carry = beforeTurn.filter(run => ['state', 'lineage', 'fork'].includes(run.sub))
  expect(carry.map(run => run.sub)).toEqual(['state', 'lineage', 'state', 'fork'])
  expect(carry.map(run => run.timeoutMs)).toEqual([2000, 2000, 2000, 2000])
})

test('fresh session does not fork', async ($, on) => {
  const clock = mock.clock(on)
  const world = forkedWorld(on, 'fresh-a')
  world.lineage = { parent: null, sharedUserTurns: 0 }

  await start($)
  await turn($, clock, 't-1', 'W6 is a window')

  expect(forks(world)).toEqual([])
  expect(subs(world)).toContain('turn')
})

test('empty transcript path is treated as fresh', async ($, on) => {
  const clock = mock.clock(on)
  const world = forkedWorld(on, 'empty-a')
  world.transcript = ''

  await start($)
  await turn($, clock, 't-1', 'W6 is a window')

  expect(subs(world)).not.toContain('lineage')
  expect(subs(world)).toContain('turn')
})

test('parent without labels does not fork', async ($, on) => {
  const clock = mock.clock(on)
  const world = forkedWorld(on, 'nolabels-a')
  world.parentLabels = 0

  await start($)
  await turn($, clock, 't-1', 'W6 is a window')

  expect(forks(world)).toEqual([])
  expect(subs(world)).toContain('turn')
})

test('lineage runs only once per session', async ($, on) => {
  const clock = mock.clock(on)
  const world = forkedWorld(on, 'once-a')

  await start($)
  await turn($, clock, 't-1', 'W6 one')
  await turn($, clock, 't-2', 'W6 two')
  await start($)
  await turn($, clock, 't-3', 'W6 three')

  expect(subs(world).filter(sub => sub === 'lineage')).toHaveLength(1)
  expect(forks(world)).toHaveLength(1)
})

test('session with turns already recorded is not checked', async ($, on) => {
  const clock = mock.clock(on)
  const world = forkedWorld(on, 'resumed-a')
  world.turns = 2

  await start($)
  await turn($, clock, 't-1', 'W6 is a window')

  expect(subs(world)).not.toContain('lineage')
})

test('lineage timeout is logged and capture continues', async ($, on) => {
  const clock = mock.clock(on)
  const world = forkedWorld(on, 'timeout-a')
  world.lineage = 'hang'

  await start($)
  await turn($, clock, 't-1', 'W6 is a window', 'Define W6')

  expect(world.logs.some(log => log.includes('lineage') && log.includes('timed out'))).toBe(true)
  expect(forks(world)).toEqual([])
  expect(subs(world)).toContain('turn')
  expect(subs(world)).toContain('apply')
})

test('fork failure is logged and capture continues', async ($, on) => {
  const clock = mock.clock(on)
  const world = forkedWorld(on, 'forkfail-a')
  world.forkError = 'session already has a glossary'

  await start($)
  await turn($, clock, 't-1', 'W6 is a window', 'Define W6')

  expect(world.logs.some(log => log.includes('session already has a glossary'))).toBe(true)
  expect(subs(world)).toContain('apply')
})

test('capture off never runs the lineage check', async ($, on) => {
  const clock = mock.clock(on)
  const world = forkedWorld(on, 'off-a')

  await start($, { isInteractive: false })
  await turn($, clock, 't-1', 'W6 is a window')

  expect(subs(world)).not.toContain('lineage')
})
