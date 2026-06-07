import { useEffect, useRef, useCallback } from 'react'
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
}

export function TerminalPanel({ id, command, args, cwd, env }: Props) {
  const containerRef = useRef<HTMLDivElement>(null)
  const termRef = useRef<Terminal | null>(null)
  const fitRef = useRef<FitAddon | null>(null)
  const startedRef = useRef(false)

  const startPty = useCallback(async (term: Terminal, _fit: FitAddon) => {
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
    const unlistenData = listen<string>(`pty-data-${id}`, (event) => {
      term.write(event.payload)
    })

    const unlistenExit = listen(`pty-exit-${id}`, () => {
      term.writeln('\r\n\x1b[33m[Process exited]\x1b[0m')
    })

    // Wait for DOM to settle so fit() gets real dimensions before starting PTY
    requestAnimationFrame(() => {
      fit.fit()
      startPty(term, fit)
    })

    // ResizeObserver to keep terminal fitted
    const ro = new ResizeObserver(() => {
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
  }, []) // intentionally empty — run once on mount

  return (
    <div
      ref={containerRef}
      className="h-full w-full"
      style={{ background: '#0f1117', padding: '4px' }}
    />
  )
}
