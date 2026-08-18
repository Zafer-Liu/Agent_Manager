/**
 * Removes explicit reasoning wrappers accidentally included in visible model
 * output.  The supported markers mirror the native backend and are kept
 * narrow so ordinary Markdown/XML remains untouched.
 */
const TAGS = ['think', 'thinking', 'analysis', 'reasoning', 'thought', 'thoughts', 'reflection']

export function stripThinkingBlocks(content: string): string {
  let result = content

  for (const tag of TAGS) result = stripAngleBlocks(result, tag)
  for (const tag of TAGS) {
    result = stripFixedBlocks(result, `[${tag}]`, `[/${tag}]`)
    result = stripFixedBlocks(result, `<|${tag}|>`, `<|/${tag}|>`)
    result = stripFixedBlocks(result, `<!-- ${tag} -->`, `<!-- /${tag} -->`)
  }

  return result.trim()
}

/** Returns the final L2 body from an older document that stored visible model planning. */
export function normalizeL2Document(content: string): string {
  const cleaned = stripThinkingBlocks(content)
  const lines = cleaned.split('\n')
  let finalHeadingIndex = -1
  for (let index = 0; index < lines.length; index += 1) {
    if (isL2FinalHeading(lines[index])) finalHeadingIndex = index
  }
  return (finalHeadingIndex === -1 ? cleaned : lines.slice(finalHeadingIndex).join('\n')).trim()
}

function isL2FinalHeading(line: string): boolean {
  const title = line.trimStart().replace(/^#+\s*/, '').toLowerCase()
  return title.startsWith('当前聚焦') || title.startsWith('current focus')
}

function stripAngleBlocks(input: string, tag: string): string {
  let result = input
  while (true) {
    const lower = result.toLowerCase()
    const opening = findAngleOpen(lower, tag, 0)
    if (!opening) break
    if (lower.slice(opening.start, opening.end).trimEnd().endsWith('/>')) {
      result = result.slice(0, opening.start) + result.slice(opening.end)
      continue
    }
    const end = findMatchingAngleClose(lower, tag, opening.end)
    if (end === undefined) {
      result = result.slice(0, opening.start)
      break
    }
    result = result.slice(0, opening.start) + result.slice(end)
  }
  return result
}

function stripFixedBlocks(input: string, open: string, close: string): string {
  let result = input
  while (true) {
    const lower = result.toLowerCase()
    const start = lower.indexOf(open)
    if (start === -1) break
    const endStart = lower.indexOf(close, start + open.length)
    if (endStart === -1) {
      result = result.slice(0, start)
      break
    }
    result = result.slice(0, start) + result.slice(endStart + close.length)
  }
  return result
}

function findAngleOpen(lower: string, tag: string, from: number): { start: number; end: number } | undefined {
  const marker = `<${tag}`
  let offset = from
  while (true) {
    const start = lower.indexOf(marker, offset)
    if (start === -1) return undefined
    const afterName = start + marker.length
    const next = lower[afterName]
    if (next === '>' || next === '/' || /\s/.test(next ?? '')) {
      const closingBracket = lower.indexOf('>', afterName)
      return { start, end: closingBracket === -1 ? lower.length : closingBracket + 1 }
    }
    offset = afterName
  }
}

function findMatchingAngleClose(lower: string, tag: string, from: number): number | undefined {
  let depth = 1
  let cursor = from
  const closingMarker = `</${tag}`
  while (cursor < lower.length) {
    const opening = findAngleOpen(lower, tag, cursor)
    const closeStart = lower.indexOf(closingMarker, cursor)
    if (closeStart === -1) return undefined
    const afterName = closeStart + closingMarker.length
    const closeNext = lower[afterName]
    if (closeNext !== '>' && !/\s/.test(closeNext ?? '')) {
      cursor = afterName
      continue
    }
    const closeBracket = lower.indexOf('>', afterName)
    const closeEnd = closeBracket === -1 ? lower.length : closeBracket + 1
    if (opening && opening.start < closeStart) {
      if (!lower.slice(opening.start, opening.end).trimEnd().endsWith('/>')) depth += 1
      cursor = opening.end
      continue
    }
    depth -= 1
    if (depth === 0) return closeEnd
    cursor = closeEnd
  }
  return undefined
}
