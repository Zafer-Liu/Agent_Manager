import ReactMarkdown from 'react-markdown'
import remarkGfm from 'remark-gfm'

interface MemoryMarkdownProps {
  content: string
  className?: string
  unwrapDocumentFence?: boolean
}

/** Shared, restrained Markdown treatment for persisted L1/L2/L3 memory. */
export function MemoryMarkdown({ content, className = '', unwrapDocumentFence = false }: MemoryMarkdownProps) {
  const renderedContent = unwrapDocumentFence ? unwrapMarkdownDocumentFence(content) : content
  return (
    <div className={`min-w-0 max-w-full break-words [overflow-wrap:anywhere] text-sm leading-6 text-gray-700 dark:text-gray-200 ${className}`}>
      <ReactMarkdown
        remarkPlugins={[remarkGfm]}
        components={{
          h1: ({ children }) => <h1 className="mb-2 mt-3 text-base font-semibold text-gray-900 first:mt-0 dark:text-gray-100">{children}</h1>,
          h2: ({ children }) => <h2 className="mb-2 mt-3 text-sm font-semibold text-gray-900 first:mt-0 dark:text-gray-100">{children}</h2>,
          h3: ({ children }) => <h3 className="mb-1.5 mt-3 text-sm font-medium text-gray-800 first:mt-0 dark:text-gray-100">{children}</h3>,
          p: ({ children }) => <p className="mb-2 last:mb-0">{children}</p>,
          ul: ({ children }) => <ul className="mb-2 list-disc space-y-1 pl-5 last:mb-0">{children}</ul>,
          ol: ({ children }) => <ol className="mb-2 list-decimal space-y-1 pl-5 last:mb-0">{children}</ol>,
          li: ({ children }) => <li className="pl-0.5">{children}</li>,
          strong: ({ children }) => <strong className="font-semibold text-gray-900 dark:text-gray-100">{children}</strong>,
          blockquote: ({ children }) => <blockquote className="my-2 border border-gray-200 bg-gray-50 px-3 py-2 text-gray-600 dark:border-gray-700 dark:bg-gray-900/45 dark:text-gray-300">{children}</blockquote>,
          code: ({ className, children, ...props }) => className?.includes('language-')
            ? <code className="font-mono text-xs text-gray-100" {...props}>{children}</code>
            : <code className="rounded bg-gray-200 px-1 py-0.5 font-mono text-xs text-gray-800 dark:bg-gray-700 dark:text-gray-100" {...props}>{children}</code>,
          pre: ({ children }) => <pre className="my-2 max-w-full overflow-x-auto rounded-md bg-gray-900 px-3 py-2 text-xs leading-5 dark:bg-black">{children}</pre>,
          table: ({ children }) => <div className="my-2 overflow-x-auto"><table className="w-full border-collapse text-left text-xs">{children}</table></div>,
          thead: ({ children }) => <thead className="bg-gray-100 text-gray-800 dark:bg-gray-700 dark:text-gray-100">{children}</thead>,
          th: ({ children }) => <th className="border border-gray-200 px-2 py-1.5 font-semibold dark:border-gray-600">{children}</th>,
          td: ({ children }) => <td className="border border-gray-200 px-2 py-1.5 align-top dark:border-gray-600">{children}</td>,
          a: ({ href, children }) => <a href={href} target="_blank" rel="noreferrer" className="text-violet-700 underline underline-offset-2 hover:text-violet-900 dark:text-violet-300 dark:hover:text-violet-200">{children}</a>,
        }}
      >
        {renderedContent}
      </ReactMarkdown>
    </div>
  )
}

/**
 * Some models wrap their entire generated document in ```markdown. That turns
 * every heading and list into one black code block, so unwrap only a document
 * fence while leaving actual fenced snippets alone.
 */
function unwrapMarkdownDocumentFence(content: string): string {
  const trimmed = content.trim()
  const opening = trimmed.match(/^```(?:markdown|md|text)?\s*\r?\n/i)
  if (!opening) return content

  const body = trimmed.slice(opening[0].length)
  const closing = body.lastIndexOf('\n```')
  const document = (closing === -1 ? body : body.slice(0, closing)).trim()
  return /^(#{1,6}\s|[-*+]\s|\d+\.\s|\|)/m.test(document) ? document : content
}
