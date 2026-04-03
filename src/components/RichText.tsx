import { Fragment } from 'react'
import { islandOpenPath } from '../lib/bridge.js'

interface RichTextProps {
  text: string
}

type Segment =
  | { type: 'text'; value: string }
  | { type: 'link'; label: string; href: string }
  | { type: 'code'; value: string }

const MARKDOWN_LINK_PATTERN = /\[([^\]]+)\]\(([^)]+)\)/g
const INLINE_CODE_PATTERN = /`([^`]+)`/g

function parseInlineSegments(text: string): Segment[] {
  const segments: Segment[] = []
  let lastIndex = 0
  for (const match of text.matchAll(INLINE_CODE_PATTERN)) {
    const index = match.index ?? 0
    if (index > lastIndex) {
      segments.push({ type: 'text', value: text.slice(lastIndex, index) })
    }
    segments.push({ type: 'code', value: match[1] })
    lastIndex = index + match[0].length
  }
  if (lastIndex < text.length) {
    segments.push({ type: 'text', value: text.slice(lastIndex) })
  }
  return segments.length > 0 ? segments : [{ type: 'text', value: text }]
}

function parseSegments(text: string): Segment[] {
  const segments: Segment[] = []
  let lastIndex = 0
  for (const match of text.matchAll(MARKDOWN_LINK_PATTERN)) {
    const index = match.index ?? 0
    if (index > lastIndex) {
      segments.push(...parseInlineSegments(text.slice(lastIndex, index)))
    }
    segments.push({
      type: 'link',
      label: match[1],
      href: match[2],
    })
    lastIndex = index + match[0].length
  }
  if (lastIndex < text.length) {
    segments.push(...parseInlineSegments(text.slice(lastIndex)))
  }
  return segments.length > 0 ? segments : [{ type: 'text', value: text }]
}

function isHttpUrl(value: string): boolean {
  return value.startsWith('http://') || value.startsWith('https://')
}

export function RichText({ text }: RichTextProps) {
  const segments = parseSegments(text)

  return (
    <>
      {segments.map((segment, index) => {
        if (segment.type === 'text') {
          return <Fragment key={`text:${index}`}>{segment.value}</Fragment>
        }
        if (segment.type === 'code') {
          return <code key={`code:${segment.value}:${index}`} className="inline-code">{segment.value}</code>
        }
        return (
          <button
            key={`link:${segment.href}:${index}`}
            type="button"
            className="inline-link"
            onClick={() => {
              if (isHttpUrl(segment.href)) {
                window.open(segment.href, '_blank', 'noopener,noreferrer')
                return
              }
              void islandOpenPath(segment.href)
            }}
            title={segment.href}
          >
            {segment.label}
          </button>
        )
      })}
    </>
  )
}
