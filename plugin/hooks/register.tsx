import type { Register } from 'claude-code'

import { checkBinary } from './bin'
import { answer, parseArgs } from './commands'
import { refreshPane, registerPane } from './pane'

export const register: Register = on => {
  on('session.start', async ($, e, next) => {
    await $.command.register({
      name: 'lecend',
      description: 'Look up the labels of this session (/lecend W6), list them, or open the pane',
    })
    if (await checkBinary($)) {
      const refreshed = await refreshPane($)
      if (!refreshed.ok) {
        $.ui.log(`lecend: pane not loaded: ${refreshed.error}`, { to: 'debug' })
      }
    }

    return next(e)
  })

  on('command.run', { command: 'lecend' }, async ($, e) => {
    const text = await answer($, parseArgs(e.args))

    return { text, isTextDisplayOnly: true }
  })

  registerPane(on)
}
