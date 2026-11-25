/** One `new` or `redefine` item of a model diff (spec §6.2). */
export type DefinitionItem = { label: string; definition: string; quote: string }

/** One `status` item of a model diff (spec §6.2). */
export type StatusItem = { label: string; status: string; quote: string }

/** The model's reply: what changed in this turn, each item with its quote. */
export type Diff = { new: DefinitionItem[]; redefine: DefinitionItem[]; status: StatusItem[] }

type Span = { from: number; to: number }

/**
 * Finds the top-level `{…}` spans of `text`, braces inside JSON strings
 * ignored (quotes count only inside an object, so prose apostrophes do not
 * matter). Returns null when an object opens and never closes: a truncated reply.
 */
function objectSpans(text: string): Span[] | null {
  const spans: Span[] = []
  let depth = 0
  let from = 0
  let inString = false
  let escaped = false
  for (let at = 0; at < text.length; at++) {
    const ch = text[at]
    if (inString) {
      if (escaped) escaped = false
      else if (ch === '\\') escaped = true
      else if (ch === '"') inString = false
    } else if (ch === '"' && depth > 0) {
      inString = true
    } else if (ch === '{') {
      if (depth === 0) from = at
      depth += 1
    } else if (ch === '}' && depth > 0) {
      depth -= 1
      if (depth === 0) spans.push({ from, to: at + 1 })
    }
  }

  return depth === 0 ? spans : null
}

function isText(value: unknown): value is string {
  return typeof value === 'string'
}

function isDefinitionItem(value: unknown): value is DefinitionItem {
  const item = value as Partial<DefinitionItem> | null
  return isText(item?.label) && isText(item.definition) && isText(item.quote)
}

function isStatusItem(value: unknown): value is StatusItem {
  const item = value as Partial<StatusItem> | null
  return isText(item?.label) && isText(item.status) && isText(item.quote)
}

/** A missing array is empty; anything else must be an array of `check` items. */
function arrayOf<T>(value: unknown, check: (item: unknown) => item is T): T[] | null {
  if (value === undefined) return []
  if (!Array.isArray(value) || !value.every(check)) return null

  return value
}

function toDiff(value: unknown): Diff | null {
  if (typeof value !== 'object' || value === null || Array.isArray(value)) return null
  const raw = value as Record<string, unknown>
  const created = arrayOf(raw.new, isDefinitionItem)
  const redefine = arrayOf(raw.redefine, isDefinitionItem)
  const status = arrayOf(raw.status, isStatusItem)
  if (created === null || redefine === null || status === null) return null

  return { new: created, redefine, status }
}

/**
 * Reads the model's reply: a bare JSON object, a fenced one, or exactly one
 * balanced object inside prose. Two top-level objects, a truncated object,
 * invalid JSON, a non-object or wrong array shapes give null.
 */
export function parseReply(text: string): Diff | null {
  const spans = objectSpans(text)
  const span = spans?.length === 1 ? spans[0] : undefined
  if (span === undefined) return null
  try {
    return toDiff(JSON.parse(text.slice(span.from, span.to)))
  } catch {
    return null
  }
}
