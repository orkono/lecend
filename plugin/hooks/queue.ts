/** A unit of background work; it handles its own errors. */
export type Task = () => Promise<void>

/** Starts `run` later, outside the dispatch that pushed the task. */
export type Schedule = (run: () => void) => void

/**
 * A FIFO with one worker: tasks run one at a time, in push order. A task that
 * throws anyway is skipped; the next one runs.
 */
export function createQueue(): { push: (task: Task, schedule: Schedule) => void } {
  const tasks: Task[] = []
  let running = false

  async function drain(): Promise<void> {
    for (let task = tasks.shift(); task !== undefined; task = tasks.shift()) {
      await task().catch(() => undefined)
    }
    running = false
  }

  function push(task: Task, schedule: Schedule): void {
    tasks.push(task)
    if (running) return
    running = true
    try {
      schedule(() => void drain())
    } catch (err) {
      running = false
      throw err
    }
  }

  return { push }
}
