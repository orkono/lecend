import { expect, mock, test } from 'claude-code/testing'
import type { ModelCompleteResult } from 'claude-code'

import { COST_NOTICE_TOKENS, buildPrompt } from './capture'
import {
  BUDGET_TOAST, GOOD_DIFF, USAGE, answered, fakeWorld, models, start, subs, turn,
} from './capture.fakes'
import type { World } from './capture.fakes'

test('buildPrompt keeps head 8 000 and tail 4 000 of a 100 KB answer', () => {
  const answer = 'W6 ' + 'h'.repeat(9_000) + 'MIDDLE' + 'm'.repeat(80_000) + 't'.repeat(10_991)
  const prompt = buildPrompt({ prompt: '', answer, knownLabels: ['OLD'], entries: [] })

  expect(answer.length).toBe(100_000)
  expect(prompt).toContain(answer.slice(0, 8_000))
  expect(prompt).toContain(answer.slice(-4_000))
  expect(prompt).not.toContain(answer.slice(0, 8_001))
  expect(prompt).not.toContain(answer.slice(-4_001))
  expect(prompt).not.toContain('MIDDLE')
})

test('buildPrompt states the rules and gives labels and entries as context', () => {
  const entries = [{ label: 'W6', definition: 'the Saturday window', status: 'approved' }]
  const prompt = buildPrompt({ prompt: 'p', answer: 'a', knownLabels: ['W6', 'OLD'], entries })

  expect(prompt).toContain('untrusted data')
  expect(prompt).toContain('redefine')
  expect(prompt).toContain('empty arrays')
  expect(prompt).toContain('verbatim')
  expect(prompt).toContain('W6, OLD')
  expect(prompt).toContain('the Saturday window')
})

test('the job sends a truncated turn to the model and the full answer to apply', async ($, on) => {
  const clock = mock.clock(on)
  const world = fakeWorld(on)
  const answer = 'W6 ' + 'h'.repeat(9_000) + 'MIDDLE' + 'm'.repeat(90_991)

  await start($)
  await turn($, clock, 't-1', answer, 'Report W6')

  expect(world.prompts[0]).not.toContain('MIDDLE')
  const apply = world.runs.find(run => run.sub === 'apply')
  expect(JSON.parse(apply?.stdin ?? '{}')).toEqual({
    turnId: 't-1', prompt: 'Report W6', answer, stoplist: [], diff: GOOD_DIFF,
  })
  expect(subs(world).at(-1)).toBe('list')
})

test('turn runs lecend turn with a 2 s timeout', async ($, on) => {
  const clock = mock.clock(on)
  const world = fakeWorld(on)

  await start($)
  await turn($, clock, 't-1', 'W6 is a window', 'Define W6')
  await turn($, clock, 't-2', 'W6 again', 'More')

  const turns = world.runs.filter(run => run.sub === 'turn')
  expect(turns.map(run => [run.stdin, run.timeoutMs])).toEqual([
    ['{"turnId":"t-1"}', 2000],
    ['{"turnId":"t-2"}', 2000],
  ])
})

const appliedPrompts = (world: World) =>
  world.runs.filter(run => run.sub === 'apply').map(run => JSON.parse(run.stdin ?? '{}').prompt)

test("prompt typed during turn t-1 does not reach t-1's job", async ($, on) => {
  const clock = mock.clock(on)
  const world = fakeWorld(on)

  await start($)
  await $.turn.start({ text: 'Define W6', turnId: 't-1' })
  await $.prompt.submit({ text: 'Typed meanwhile', wait: false, turnId: 't-1', origin: { kind: 'composer' } })
  await $.turn.complete({ answer: 'W6 is a window', durationMs: 1, isAborted: false, turnId: 't-1', reason: 'answer' })
  await clock.settle()

  expect(appliedPrompts(world)).toEqual(['Define W6'])
})

test('continuation turn does not reuse an earlier prompt', async ($, on) => {
  const clock = mock.clock(on)
  const world = fakeWorld(on)

  await start($)
  await turn($, clock, 't-1', 'W6 is a window', 'Define W6')
  await turn($, clock, 't-2', 'W6 again', '')
  await turn($, clock, 't-3', 'W6 once more', '/lecend W6')
  await $.turn.complete({ answer: 'W6 no start', durationMs: 1, isAborted: false, turnId: 't-4', reason: 'answer' })
  await clock.settle()

  expect(appliedPrompts(world)).toEqual(['Define W6', '', '', ''])
})

test('parseReply failures never run apply', async ($, on) => {
  const clock = mock.clock(on)
  const world = fakeWorld(on)
  const bad = ['Sure: {"new": [', `${JSON.stringify(GOOD_DIFF)} ${JSON.stringify(GOOD_DIFF)}`, '[]']
  let index = 0
  world.reply = () => answered(bad[index++] ?? '')

  await start($)
  for (const id of ['t-1', 't-2', 't-3']) {
    await turn($, clock, id, 'W6 is a window')
  }

  expect(models(world)).toBe(3)
  expect(subs(world)).not.toContain('apply')
  expect(world.logs.filter(line => line.includes('no single JSON diff'))).toHaveLength(3)
})

test('subagent turn is ignored', async ($, on) => {
  const clock = mock.clock(on)
  const world = fakeWorld(on)

  await start($)
  const before = world.runs.length
  await $.turn.complete({
    answer: 'W6 x', durationMs: 1, isAborted: false, turnId: 't-1', reason: 'answer', agentId: 'a1',
  })
  await $.turn.complete({ answer: 'W6 x', durationMs: 1, isAborted: true, turnId: 't-2', reason: 'aborted' })
  await clock.settle()

  expect(world.runs.length).toBe(before)
  expect(models(world)).toBe(0)
})

test('turn hook returns before the model call resolves', async ($, on) => {
  const clock = mock.clock(on)
  const world = fakeWorld(on)
  world.reply = () => new Promise<ModelCompleteResult>(() => {})

  await start($)
  const first = await turn($, clock, 't-1', 'W6 one')
  const second = await turn($, clock, 't-2', 'W6 two')

  expect(first.text).toBe('W6 one')
  expect(second.text).toBe('W6 two')
  expect(models(world)).toBe(1)
  expect(subs(world).filter(sub => sub === 'turn')).toHaveLength(2)
})

test('jobs run in turn order', async ($, on) => {
  const clock = mock.clock(on)
  const world = fakeWorld(on)
  world.reply = async prompt => {
    if (prompt.includes('first W6')) await clock.sleep(1000)
    world.events.push('model-done')
    return answered(JSON.stringify(GOOD_DIFF))
  }

  await start($)
  await turn($, clock, 't-1', 'first W6')
  await turn($, clock, 't-2', 'second W6')
  await clock.advance(1000)

  const order = world.events.filter(e => /^(model|model-done|apply:.*)$/.test(e))
  expect(order).toEqual(['model', 'model-done', 'apply:t-1', 'model', 'model-done', 'apply:t-2'])
})

test('budget stops calls and toasts once', { options: { budgetPerSession: 2 } }, async ($, on) => {
  const clock = mock.clock(on)
  const world = fakeWorld(on)

  await start($)
  for (const id of ['t-1', 't-2', 't-3', 't-4']) {
    await turn($, clock, id, 'W6 again')
  }

  expect(models(world)).toBe(2)
  expect(world.toasts).toEqual([BUDGET_TOAST])
})

test('usage bumped before each model call, also when the call fails', async ($, on) => {
  const clock = mock.clock(on)
  const world = fakeWorld(on)
  world.reply = () => ({
    isAnswered: false, reason: 'api-error', status: 529, error: 'overloaded', usage: USAGE,
  }) as ModelCompleteResult

  await start($)
  await turn($, clock, 't-1', 'W6 one')
  await turn($, clock, 't-2', 'W6 two')

  const order = world.events.filter(e => e === 'bin:usage' || e === 'model')
  expect(order).toEqual(['bin:usage', 'model', 'bin:usage', 'model'])
  expect(world.state.modelCalls).toBe(2)
  expect(subs(world)).not.toContain('apply')
})

test('cost notice written once across two sessions, before the first model call', async ($, on) => {
  const clock = mock.clock(on)
  const session = mock.session(on)
  const world = fakeWorld(on)
  const appendedAtModel: number[] = []
  world.reply = () => {
    appendedAtModel.push(session.appended().length)
    return answered(JSON.stringify(GOOD_DIFF))
  }

  await start($)
  await turn($, clock, 't-1', 'W6 one')
  world.session = 'sess-2'
  await start($)
  await turn($, clock, 't-2', 'W6 two')

  const text =
    `lecend captures session labels by sending each turn's text to haiku on your Claude account ` +
    '(and count toward your quota) ' +
    `(roughly ${COST_NOTICE_TOKENS} tokens per captured turn; budget 200 calls per session). ` +
    '/lecend off stops it for this session; set capture=false in the plugin settings to stop it everywhere.'
  const rows = session.appended().map(({ message }) => ({ type: message.type, content: message.content }))
  expect(rows).toEqual([{ type: 'system', content: [{ type: 'text', text }] }])
  expect(COST_NOTICE_TOKENS).toBe(800)
  expect(appendedAtModel).toEqual([1, 1])
  expect(subs(world).filter(sub => sub === 'notice')).toHaveLength(2)
})

test('headless session never calls the model', async ($, on) => {
  const clock = mock.clock(on)
  const world = fakeWorld(on)

  await start($, { isInteractive: false })
  await turn($, clock, 't-1', 'W6 one')

  expect(subs(world)).toEqual(['version', 'list'])
  expect(models(world)).toBe(0)
})

test('isInteractive missing never calls the model', async ($, on) => {
  const clock = mock.clock(on)
  const world = fakeWorld(on)

  await start($, {})
  await turn($, clock, 't-1', 'W6 one')

  expect(subs(world)).toEqual(['version', 'list'])
  expect(models(world)).toBe(0)
})

test('missing binary: no process run after session.start and no model call on turns', async ($, on) => {
  const clock = mock.clock(on)
  const world = fakeWorld(on)
  world.missing = true

  await start($)
  await turn($, clock, 't-1', 'W6 one')
  await turn($, clock, 't-2', 'W6 two')

  expect(subs(world)).toEqual(['version'])
  expect(models(world)).toBe(0)
})

test('capture=false setting never calls the model', { options: { capture: false } }, async ($, on) => {
  const clock = mock.clock(on)
  const world = fakeWorld(on)

  await start($)
  await turn($, clock, 't-1', 'W6 one')

  expect(subs(world)).toEqual(['version', 'list'])
  expect(models(world)).toBe(0)
})

test('/lecend off stops capture on the next job', async ($, on) => {
  const clock = mock.clock(on)
  const world = fakeWorld(on)

  await start($)
  await turn($, clock, 't-1', 'W6 one')
  await $.command.run({
    command: 'lecend',
    args: 'off',
    origin: { kind: 'composer' },
    presentation: { isFullscreen: false, columns: 80 },
  })
  const before = world.runs.length
  await turn($, clock, 't-2', 'W6 two')

  expect(models(world)).toBe(1)
  expect(world.runs.slice(before).map(run => run.sub)).toEqual(['turn', 'state'])
})

test('stoplist and model settings reach scan, apply and the model call', {
  options: { stoplist: ' FOO, ,bar ,', model: 'sonnet' },
}, async ($, on) => {
  const clock = mock.clock(on)
  const world = fakeWorld(on)

  await start($)
  await turn($, clock, 't-1', 'W6 one')

  for (const sub of ['scan', 'apply']) {
    const run = world.runs.find(r => r.sub === sub)
    expect(JSON.parse(run?.stdin ?? '{}').stoplist).toEqual(['FOO', 'bar'])
  }
  expect(world.asked).toEqual(['sonnet:low'])
})

test('failed notice append resets the flag and stops capture', async ($, on) => {
  const clock = mock.clock(on)
  const world = fakeWorld(on)
  on('session.append', () => ({ deny: 'rows are refused here' }))

  await start($)
  await turn($, clock, 't-1', 'W6 one')
  const before = world.runs.length
  await turn($, clock, 't-2', 'W6 two')

  const notices = world.runs.filter(run => run.sub === 'notice').map(run => run.argv.at(-1))
  expect(notices).toEqual(['--check-and-set', '--reset'])
  expect(world.events).not.toContain('notice-set')
  expect(models(world)).toBe(0)
  expect(world.runs.length).toBe(before)
  expect(world.logs.some(line => line.includes('cost notice not shown'))).toBe(true)
})

test('queued jobs are dropped on a new session.start', async ($, on) => {
  const clock = mock.clock(on)
  const world = fakeWorld(on)
  world.reply = async prompt => {
    if (prompt.includes('first W6')) await clock.sleep(1000)
    return answered(JSON.stringify(GOOD_DIFF))
  }

  await start($)
  await turn($, clock, 't-1', 'first W6')
  await turn($, clock, 't-2', 'second W6')
  world.session = 'sess-2'
  await start($)
  await clock.advance(1000)

  expect(models(world)).toBe(1)
  expect(world.events.filter(event => event.startsWith('apply:'))).toEqual(['apply:t-1'])
})
