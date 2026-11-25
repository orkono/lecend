/** One glossary entry as `lecend list --json` prints it. */
export type Entry = {
  label: string
  definition: string
  status: string | null
  previously: string | null
  updatedTurn: number
  pinned: boolean
}

declare module 'claude-code' {
  interface PluginState {
    lecend: { entries: Shaped<Entry[]> }
  }
}
