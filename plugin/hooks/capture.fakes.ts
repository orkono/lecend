/** Test doubles for capture.test.ts: a fake `lecend` binary, model, log and toast. */
import type { ModelCompleteResult, On, SessionInfo, SessionStartInput } from 'claude-code'
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
const SCAN = { shouldCall: true, knownLabels: [], candidates: ['W6'], mentioned: [], entries: [] }
export const BUDGET_TOAST = 'lecend: capture budget reached for this session'

type Run = { sub: string; argv: readonly string[]; stdin?: string; timeoutMs?: number }
type Reply = (prompt: string) => ModelCompleteResult | Promise<ModelCompleteResult>

/** A fake `lecend` binary, model, log and toast sink; `events` keeps their order. */
export type World = {
  /** Starts the turn the next idle prompt.submit starts, as the engine would. */
  startsTurn?: () => Promise<unknown>
  runs: Run[]
  events: string[]
  prompts: string[]
  asked: string[]
  logs: string[]
  toasts: string[]
  session: string
  /** `CLAUDE_CODE_SESSION_ID` in the hook environment; unset, `session`. */
  envSession?: string
  /** Makes `$.ui.toast` throw this error. */
  toastError?: string
  /** Known labels `lecend scan` reports as mentioned in a turn. */
  mentioned: string[]
  state: { captureOff: boolean; modelCalls: number }
  /** Turns the fake binary has recorded for the current session. */
  turns: number
  /** `session.info().transcriptPath`; empty skips the lineage check. */
  transcript: string
  /** What `lecend lineage` answers; `'hang'` makes the call time out. */
  lineage: { parent: string | null; sharedUserTurns: number } | 'hang'
  /** Label count `lecend state` reports for any session but the current one. */
  parentLabels: number
  /** Makes `lecend fork` fail with this error. */
  forkError?: string
  missing: boolean
  reply: Reply
}

export const answered = (text: string): ModelCompleteResult => ({ isAnswered: true, text, usage: USAGE })

function binaryReply(world: World, sub: string, argv: readonly string[], stdin?: string): string {
  switch (sub) {
    case 'version':
      return '{"version":"0.1.0"}'
    case 'turn':
      world.turns += 1
      return JSON.stringify({ turnIndex: world.turns })
    case 'state': {
      const own = argv[argv.indexOf('--session') + 1] === world.session
      const labels = own ? 0 : world.parentLabels
      const turns = own ? world.turns : 5
      const forkDone = own && world.events.some(event => event.startsWith('fork:'))

      return JSON.stringify({ ...world.state, turns, labels, forkedFrom: null, forkTurn: null, forkDone })
    }
    case 'lineage':
      return JSON.stringify(world.lineage)
    case 'fork':
      if (world.forkError !== undefined) return JSON.stringify({ error: world.forkError })
      world.events.push(`fork:${argv[argv.indexOf('--from') + 1]}@${argv[argv.indexOf('--at-turn') + 1]}`)
      return '{"copied":3,"already":false}'
    case 'scan': {
      const { mentioned } = world
      if (JSON.parse(stdin ?? '{}').mentionsOnly === true) {
        return JSON.stringify({ ...SCAN, shouldCall: mentioned.length > 0, candidates: [], mentioned })
      }
      return JSON.stringify({ ...SCAN, mentioned })
    }
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
    mentioned: [],
    state: { captureOff: false, modelCalls: 0 },
    turns: 0,
    transcript: '',
    lineage: { parent: null, sharedUserTurns: 0 },
    parentLabels: 0,
    missing: false,
    reply: () => answered(JSON.stringify(GOOD_DIFF)),
  }
  on('command.register', () => ({ value: { command: 'lecend' } }))
  on('session.start', (_, e) => ({ cwd: e.cwd }))
  // As the engine does, a prompt that starts a turn resolves after that turn started.
  on('prompt.submit', async (_, e) => {
    const startTurn = world.startsTurn
    world.startsTurn = undefined
    if (startTurn !== undefined) await startTurn()
    return { text: e.text }
  })
  on('turn.start', (_, e) => ({ turnId: e.turnId }))
  on('turn.complete', (_, e) => ({ text: e.answer }))
  on('session.info', () => ({ value: { transcriptPath: world.transcript } as SessionInfo }))
  on('process.hookEnv', () => ({
    value: { CLAUDE_PLUGIN_DATA: '/data', CLAUDE_CODE_SESSION_ID: world.envSession ?? world.session },
  }))
  on('session.id', () => ({ value: world.session }))
  on('process.run', (_, e) => {
    const sub = e.argv[1] ?? ''
    world.runs.push({ sub, argv: e.argv, stdin: e.init?.stdin, timeoutMs: e.init?.timeoutMs })
    world.events.push(`bin:${sub}`)
    if (sub === 'lineage' && world.lineage === 'hang') return { deny: 'process timed out' }
    const missing = world.missing && sub === 'version'
    const stdout = missing ? '' : binaryReply(world, sub, e.argv, e.init?.stdin)
    const failed = stdout.startsWith('{"error"')
    const value = { exitCode: missing ? 127 : failed ? 1 : 0, stdout, stderr: '' }

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
    if (world.toastError !== undefined) throw new Error(world.toastError)
    world.toasts.push(e.text)
    return { value: undefined }
  })

  return world
}

/** Starts the session; `{}` leaves `isInteractive` out. */
export async function start($: Engine, fields: { isInteractive?: boolean } = { isInteractive: true }) {
  await $.session.start({ cwd: '/', surface: 'terminal', ...fields } as SessionStartInput)
}

/**
 * A notification's turn: its `prompt.submit` (origin `kind`, session idle),
 * its start and answer, then the background job runs.
 */
export async function notifiedTurn(
  $: Engine,
  clock: MockClock,
  world: World,
  turnId: string,
  answer: string,
  kind: 'task-notification' | 'sdk' = 'task-notification',
) {
  const text = '<task-notification>HEAD 1992ad0 W9</task-notification>'
  world.startsTurn = () => $.turn.start({ text, turnId })
  await $.prompt.submit({ text, wait: false, origin: { kind } })
  await $.turn.complete({ answer, durationMs: 1, isAborted: false, turnId, reason: 'answer' })
  await clock.settle()
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
