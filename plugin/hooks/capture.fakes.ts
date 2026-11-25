/** Test doubles for capture.test.ts: a fake `lecend` binary, model, log and toast. */
import type { ModelCompleteResult, On, SessionStartInput } from 'claude-code'
import type { Engine, MockClock } from 'claude-code/testing'

export const USAGE = {
  input_tokens: 10,
  output_tokens: 10,
  cache_read_input_tokens: 0,
  cache_creation_input_tokens: 0,
}
export const GOOD_DIFF = {
  new: [{ label: 'W6', definition: 'a window', quote: 'W6 is a window' }],
  redefine: [],
  status: [],
}
const SCAN = { shouldCall: true, knownLabels: [], candidates: ['W6'], entries: [] }
export const BUDGET_TOAST = 'lecend: capture budget reached for this session'

type Run = { sub: string; argv: readonly string[]; stdin?: string; timeoutMs?: number }
type Reply = (prompt: string) => ModelCompleteResult | Promise<ModelCompleteResult>

/** A fake `lecend` binary, model, log and toast sink; `events` keeps their order. */
export type World = {
  runs: Run[]
  events: string[]
  prompts: string[]
  asked: string[]
  logs: string[]
  toasts: string[]
  session: string
  state: { captureOff: boolean; modelCalls: number }
  missing: boolean
  reply: Reply
}

export const answered = (text: string): ModelCompleteResult => ({ isAnswered: true, text, usage: USAGE })

function binaryReply(world: World, sub: string, argv: readonly string[], stdin?: string): string {
  switch (sub) {
    case 'version':
      return '{"version":"0.1.0"}'
    case 'turn':
      return '{"turnIndex":1}'
    case 'state':
      return JSON.stringify({ ...world.state, forkedFrom: null, forkTurn: null, forkDone: false })
    case 'scan':
      return JSON.stringify(SCAN)
    case 'usage':
      world.state.modelCalls += 1
      return JSON.stringify({ modelCalls: world.state.modelCalls })
    case 'notice': {
      if (argv.includes('--reset')) {
        world.events = world.events.filter(event => event !== 'notice-set')
        return '{"shown":false}'
      }
      const shown = world.events.includes('notice-set')
      world.events.push('notice-set')
      return JSON.stringify({ shown })
    }
    case 'apply':
      world.events.push(`apply:${JSON.parse(stdin ?? '{}').turnId}`)
      return '{"applied":1,"rejected":[]}'
    case 'capture':
      world.state.captureOff = argv.includes('--off')
      return JSON.stringify({ captureOff: world.state.captureOff })
    default:
      return '[]'
  }
}

export function fakeWorld(on: On): World {
  const world: World = {
    runs: [],
    events: [],
    prompts: [],
    asked: [],
    logs: [],
    toasts: [],
    session: 'sess-1',
    state: { captureOff: false, modelCalls: 0 },
    missing: false,
    reply: () => answered(JSON.stringify(GOOD_DIFF)),
  }
  on('command.register', () => ({ value: { command: 'lecend' } }))
  on('session.start', (_, e) => ({ cwd: e.cwd }))
  on('prompt.submit', (_, e) => ({ text: e.text }))
  on('turn.start', (_, e) => ({ turnId: e.turnId }))
  on('turn.complete', (_, e) => ({ text: e.answer }))
  on('process.hookEnv', () => ({
    value: { CLAUDE_PLUGIN_DATA: '/data', CLAUDE_CODE_SESSION_ID: world.session },
  }))
  on('process.run', (_, e) => {
    const sub = e.argv[1] ?? ''
    world.runs.push({ sub, argv: e.argv, stdin: e.init?.stdin, timeoutMs: e.init?.timeoutMs })
    world.events.push(`bin:${sub}`)
    const missing = world.missing && sub === 'version'
    const stdout = missing ? '' : binaryReply(world, sub, e.argv, e.init?.stdin)
    const value = { exitCode: missing ? 127 : 0, stdout, stderr: '' }

    return { value: { ...value, isStdoutTruncated: false, isStderrTruncated: false } }
  })
  on('model.complete', async (_, e) => {
    world.events.push('model')
    world.prompts.push(e.prompt)
    world.asked.push(`${e.model}:${e.effort}`)

    return { value: await world.reply(e.prompt) }
  })
  on('ui.log', (_, e) => {
    if (e.to === 'debug') world.logs.push(e.text)
    return { value: undefined }
  })
  on('ui.toast', (_, e) => {
    world.toasts.push(e.text)
    return { value: undefined }
  })

  return world
}

/** Starts the session; `{}` leaves `isInteractive` out. */
export async function start($: Engine, fields: { isInteractive?: boolean } = { isInteractive: true }) {
  await $.session.start({ cwd: '/', surface: 'terminal', ...fields } as SessionStartInput)
}

/** One main-loop turn: its start with `prompt`, its answer, then the background job runs. */
export async function turn($: Engine, clock: MockClock, turnId: string, answer: string, prompt = 'go') {
  await $.turn.start({ text: prompt, turnId })
  const done = await $.turn.complete({
    answer, durationMs: 1, isAborted: false, turnId, reason: 'answer',
  })
  await clock.settle()

  return done
}

export const subs = (world: World) => world.runs.map(run => run.sub)
export const models = (world: World) => world.events.filter(event => event === 'model').length
