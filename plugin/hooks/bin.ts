import type { EngineInterface } from 'claude-code'

/** What every `/lecend` answer is when the binary cannot be started. */
export const INSTALL_TEXT =
  'lecend binary not found on PATH. Install: brew install --cask orkono/tap/lecend  (or: cargo install lecend). Then restart the session.'

export type LecendResult =
  | { ok: true; json: any; text: string }
  | { ok: false; error: string; missing: boolean }

// Whether `lecend version` ran, probed at session.start; undefined until then.
let binaryFound: boolean | undefined

/** Runs `lecend version` and caches whether the binary is on PATH. */
export async function checkBinary($: EngineInterface): Promise<boolean> {
  try {
    const ran = await $.process.run(['lecend', 'version'], { timeoutMs: 5000 })
    binaryFound = ran.exitCode === 0
  } catch {
    binaryFound = false
  }

  return binaryFound
}

/** The cached probe, probing once when session.start has not run yet. */
export async function hasBinary($: EngineInterface): Promise<boolean> {
  return binaryFound ?? (await checkBinary($))
}

/** The current session's id: the environment's, else the engine's. */
export async function currentSessionId($: EngineInterface): Promise<string> {
  const env = await $.process.hookEnv()

  return env.CLAUDE_CODE_SESSION_ID || (await $.session.id())
}

async function sessionArgs($: EngineInterface, session?: string): Promise<string[]> {
  const env = await $.process.hookEnv()
  const data = env.CLAUDE_PLUGIN_DATA
  if (data === undefined || data === '') {
    throw new Error('CLAUDE_PLUGIN_DATA is not set')
  }

  return ['--db', `${data}/lecend.db`, '--session', session ?? (await currentSessionId($))]
}

function parseJson(text: string): unknown {
  try {
    return JSON.parse(text)
  } catch {
    return undefined
  }
}

function errorOf(stdout: string, stderr: string, exitCode: number): string {
  const json = parseJson(stdout) as { error?: unknown } | undefined
  if (typeof json?.error === 'string') {
    return json.error
  }
  const said = stderr.trim() || stdout.trim()

  return said === '' ? `exited with code ${exitCode}` : said
}

/**
 * Runs `lecend <sub> --db … --session … [rest…]`, where `args` is
 * `[sub, ...rest]`. `missing` is true when the process could not start.
 * `options.session` runs it against another session than the current one.
 */
export async function runLecend(
  $: EngineInterface,
  args: string[],
  stdin?: unknown,
  timeoutMs = 5000,
  options: { session?: string } = {},
): Promise<LecendResult> {
  const [sub, ...rest] = args
  if (sub === undefined) {
    return { ok: false, error: 'no subcommand', missing: false }
  }
  let argv: string[]
  try {
    argv = ['lecend', sub, ...(await sessionArgs($, options.session)), ...rest]
  } catch (err) {
    return { ok: false, error: String((err as Error).message), missing: false }
  }
  const input =
    stdin === undefined ? undefined : typeof stdin === 'string' ? stdin : JSON.stringify(stdin)
  try {
    const ran = await $.process.run(argv, { stdin: input, timeoutMs })
    if (ran.exitCode !== 0) {
      return { ok: false, error: errorOf(ran.stdout, ran.stderr, ran.exitCode), missing: false }
    }

    return { ok: true, json: parseJson(ran.stdout), text: ran.stdout.trimEnd() }
  } catch (err) {
    const message = String((err as Error)?.message ?? err)
    // A run that started but outlived its timeout rejects too; only a start
    // failure means the binary is missing.
    return { ok: false, error: message, missing: !/time/i.test(message) }
  }
}
