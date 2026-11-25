import { expect, test } from 'claude-code/testing'

import { parseReply } from './reply'

const DIFF = {
  new: [{ label: 'W6', definition: 'the maintenance window', quote: 'W6 is the window' }],
  redefine: [],
  status: [{ label: 'NEW', status: 'unreachable', quote: 'NEW is unreachable' }],
}
const JSON_TEXT = JSON.stringify(DIFF)

test('parseReply accepts a bare object', () => {
  expect(parseReply(JSON_TEXT)).toEqual(DIFF)
  expect(parseReply(`  \n${JSON_TEXT}\n`)).toEqual(DIFF)
})

test('parseReply accepts a fenced object', () => {
  expect(parseReply('```json\n' + JSON_TEXT + '\n```')).toEqual(DIFF)
  expect(parseReply('```\n' + JSON_TEXT + '\n```')).toEqual(DIFF)
})

test('parseReply accepts one object wrapped in prose', () => {
  expect(parseReply(`Here is the diff:\n${JSON_TEXT}\nDone.`)).toEqual(DIFF)
})

test('parseReply keeps braces inside strings', () => {
  const diff = {
    new: [{ label: 'D4', definition: 'test {a} "b"', quote: 'D4 checks {x}' }],
    redefine: [],
    status: [],
  }
  expect(parseReply(`ok: ${JSON.stringify(diff)}`)).toEqual(diff)
})

test('parseReply fills missing arrays with empty ones', () => {
  expect(parseReply('{"new":[]}')).toEqual({ new: [], redefine: [], status: [] })
})

test('parseReply rejects truncated JSON, two objects and non-objects', () => {
  const rejected = [
    JSON_TEXT.slice(0, -10),
    `Diff: ${JSON_TEXT.slice(0, 40)}`,
    `${JSON_TEXT}\n${JSON_TEXT}`,
    `first ${JSON_TEXT} then {"new":[]}`,
    '[]',
    '42',
    '"text"',
    'null',
    'no JSON here',
    '',
  ]
  for (const text of rejected) {
    expect(parseReply(text)).toBeNull()
  }
})

test('parseReply rejects wrong array shapes', () => {
  const rejected = [
    '{"new":{}}',
    '{"new":[{"label":"W6","definition":"x"}]}',
    '{"new":[{"label":"W6","definition":3,"quote":"W6"}]}',
    '{"status":[{"label":"W6","quote":"W6"}]}',
    '{"redefine":["W6"]}',
    '{"status":[null]}',
  ]
  for (const text of rejected) {
    expect(parseReply(text)).toBeNull()
  }
})
