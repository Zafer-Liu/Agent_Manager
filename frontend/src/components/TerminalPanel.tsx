import { useEffect, useRef, useCallback, useState } from 'react'
import { Terminal } from '@xterm/xterm'
import { FitAddon } from '@xterm/addon-fit'
import { WebLinksAddon } from '@xterm/addon-web-links'
import { invoke } from '@tauri-apps/api/core'
import { listen } from '@tauri-apps/api/event'
import '@xterm/xterm/css/xterm.css'

interface Props {
  id: string          // unique session id (agentId)
  command: string
  args: string[]
  cwd: string
  env: Record<string, string>
  active: boolean
  clearVersion: number
}

export function TerminalPanel({ id, command, args, cwd, env, active, clearVersion }: Props) {
  const containerRef = useRef<HTMLDivElement>(null)
  const termRef = useRef<Terminal | null>(null)
  const fitRef = useRef<FitAddon | null>(null)
  const startedRef = useRef(false)
  const activeRef = useRef(active)
  const clearVersionRef = useRef(clearVersion)
  const [showSearch, setShowSearch] = useState(false)
  const [searchTerm, setSearchTerm] = useState('')

  const startPty = useCallback(async (term: Terminal) => {
    const { cols, rows } = term
    term.writeln(`\x1b[90mStarting: ${command} ${args.join(' ')}\x1b[0m`)
    // Show resolved exe+args so we can see what Rust actually runs
    try {
      const debug = await invoke<string>('pty_resolve_debug', { command })
      term.writeln(`\x1b[90mResolved: ${debug}\x1b[0m`)
    } catch { /* ignore */ }
    try {
      await invoke('pty_start', {
        id,
        command,
        args,
        cwd,
        envVars: env,
        cols,
        rows,
      })
      // Send an initial resize after a brief delay to trigger TUI redraw
      setTimeout(() => {
        invoke('pty_resize', { id, cols: term.cols, rows: term.rows }).catch(() => {})
      }, 300)
    } catch (e) {
      term.writeln(`\r\n\x1b[31mFailed to start PTY: ${e}\x1b[0m`)
    }
  }, [id, command, args, cwd, env])

  useEffect(() => {
    if (!containerRef.current || startedRef.current) return
    startedRef.current = true

    const term = new Terminal({
      theme: {
        background: '#0f1117',
        foreground: '#e2e8f0',
        cursor: '#60a5fa',
        cursorAccent: '#0f1117',
        selectionBackground: '#3b82f640',
        black: '#1e293b',
        red: '#f87171',
        green: '#4ade80',
        yellow: '#facc15',
        blue: '#60a5fa',
        magenta: '#c084fc',
        cyan: '#22d3ee',
        white: '#e2e8f0',
        brightBlack: '#475569',
        brightRed: '#fca5a5',
        brightGreen: '#86efac',
        brightYellow: '#fde047',
        brightBlue: '#93c5fd',
        brightMagenta: '#d8b4fe',
        brightCyan: '#67e8f9',
        brightWhite: '#f8fafc',
      },
      fontFamily: '"Cascadia Code", "JetBrains Mono", "Fira Code", Consolas, monospace',
      fontSize: 13,
      lineHeight: 1.4,
      cursorBlink: true,
      cursorStyle: 'block',
      scrollback: 5000,
      allowProposedApi: true,
    })

    const fit = new FitAddon()
    const links = new WebLinksAddon()
    term.loadAddon(fit)
    term.loadAddon(links)
    term.open(containerRef.current!)
    fit.fit()

    // Custom keyboard shortcuts for copy/paste/search
    term.attachCustomKeyEventHandler((e) => {
      // Ctrl+Shift+C: Copy selection
      if (e.type === 'keydown' && e.ctrlKey && e.shiftKey && e.key === 'C') {
        const sel = term.getSelection()
        if (sel) {
          navigator.clipboard.writeText(sel).catch(() => {})
        }
        return false
      }
      // Ctrl+Shift+V: Paste
      if (e.type === 'keydown' && e.ctrlKey && e.shiftKey && e.key === 'V') {
        navigator.clipboard.readText().then(text => {
          invoke('pty_write', { id, data: text }).catch(() => {})
        }).catch(() => {})
        return false
      }
      // Ctrl+Shift+F: Toggle search bar
      if (e.type === 'keydown' && e.ctrlKey && e.shiftKey && e.key === 'F') {
        setShowSearch(v => !v)
        return false
      }
      return true
    })

    termRef.current = term
    fitRef.current = fit

    // Send keyboard input to PTY
    term.onData((data) => {
      invoke('pty_write', { id, data }).catch(() => {})
    })

    // Resize PTY when terminal resizes
    term.onResize(({ cols, rows }) => {
      invoke('pty_resize', { id, cols, rows }).catch(() => {})
    })

    // Receive PTY output from Tauri events
    const unlistenData = listen<number[]>(`pty-data-${id}`, (event) => {
      term.write(Uint8Array.from(event.payload))
    })

    const unlistenExit = listen(`pty-exit-${id}`, () => {
      term.writeln('\r\n\x1b[33m[Process exited]\x1b[0m')
    })

    // Wait for DOM to settle so fit() gets real dimensions before starting PTY
    requestAnimationFrame(() => {
      if (activeRef.current) fit.fit()
      startPty(term)
    })

    // ResizeObserver to keep terminal fitted
    const ro = new ResizeObserver(() => {
      if (!activeRef.current) return
      fit.fit()
      const { cols, rows } = term
      invoke('pty_resize', { id, cols, rows }).catch(() => {})
    })
    ro.observe(containerRef.current!)

    return () => {
      ro.disconnect()
      unlistenData.then(fn => fn())
      unlistenExit.then(fn => fn())
      invoke('pty_stop', { id }).catch(() => {})
      term.dispose()
      startedRef.current = false
    }
    // A PTY session is owned by this component mount and must not restart when
    // parent props receive equivalent object/array identities.
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [])

  useEffect(() => {
    activeRef.current = active
    if (!active) return

    const frame = requestAnimationFrame(() => {
      const term = termRef.current
      const fit = fitRef.current
      if (!term || !fit) return
      fit.fit()
      invoke('pty_resize', { id, cols: term.cols, rows: term.rows }).catch(() => {})
      term.focus()
    })
    return () => cancelAnimationFrame(frame)
  }, [active, id])

  useEffect(() => {
    if (clearVersion === clearVersionRef.current) return
    clearVersionRef.current = clearVersion
    const term = termRef.current
    if (!term) return
    term.clear()
    term.write('\x1b[2J\x1b[H')
  }, [clearVersion])

  return (
    <div className="relative h-full w-full" style={{ background: '#0f1117' }}>
      {/* Toolbar */}
      <div className="absolute top-1 right-1 z-10 flex items-center gap-1">
        <button
          onClick={() => {
            const sel = termRef.current?.getSelection()
            if (sel) navigator.clipboard.writeText(sel).catch(() => {})
          }}
          title="Copy (Ctrl+Shift+C)"
          className="rounded bg-gray-800/80 px-1.5 py-0.5 text-[10px] text-gray-300 hover:bg-gray-700"
        >
          Copy
        </button>
        <button
          onClick={() => {
            navigator.clipboard.readText().then(text => {
              invoke('pty_write', { id, data: text }).catch(() => {})
            }).catch(() => {})
          }}
          title="Paste (Ctrl+Shift+V)"
          className="rounded bg-gray-800/80 px-1.5 py-0.5 text-[10px] text-gray-300 hover:bg-gray-700"
        >
          Paste
        </button>
        <button
          onClick={() => setShowSearch(v => !v)}
          title="Search (Ctrl+Shift+F)"
          className="rounded bg-gray-800/80 px-1.5 py-0.5 text-[10px] text-gray-300 hover:bg-gray-700"
        >
          Search
        </button>
      </div>

      {/* Search bar */}
      {showSearch && (
        <div className="absolute top-7 right-1 z-10 flex items-center gap-1.5 rounded-lg bg-gray-800 border border-gray-600 px-2 py-1 shadow-lg">
          <input
            autoFocus
            value={searchTerm}
            onChange={e => setSearchTerm(e.target.value)}
            onKeyDown={e => {
              if (e.key === 'Escape') { setShowSearch(false); setSearchTerm('') }
              if (e.key === 'Enter') {
                // Simple: find in scrollback by writing a note
                const term = termRef.current
                if (term && searchTerm) {
                  // Use terminal's findNext if available via addon
                  // Fallback: just inform user
                }
              }
            }}
            placeholder="Find..."
            className="bg-transparent text-white text-xs outline-none w-32 placeholder-gray-500"
          />
          <button onClick={() => { setShowSearch(false); setSearchTerm('') }} className="text-gray-400 hover:text-white">
            ✕
          </button>
        </div>
      )}

      <div
        ref={containerRef}
        className="h-full w-full"
        style={{ background: '#0f1117', padding: '4px' }}
      />
    </div>
  )
}
