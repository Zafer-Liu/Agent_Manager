import { useEffect, useRef, useCallback } from 'react'
import { invoke } from '@tauri-apps/api/core'
import { listen } from '@tauri-apps/api/event'

export interface OpenTab {
  agentId: string
  label: string
  kind: 'ui' | 'terminal'
  port?: number
  token?: string
  command?: string
  args?: string[]
  cwd?: string
  env?: Record<string, string>
}

export function NativeWebviewPanel({ tab, active }: { tab: OpenTab; active: boolean }) {
  const containerRef = useRef<HTMLDivElement>(null)
  const openedRef = useRef(false)
  const activeRef = useRef(active)
  const base = `http://127.0.0.1:${tab.port}/`
  const url = tab.token ? `${base}#token=${encodeURIComponent(tab.token)}` : base

  const getBounds = useCallback(() => {
    const rect = containerRef.current?.getBoundingClientRect()
    if (!rect) return null
    return {
      x: rect.left,
      y: rect.top,
      width: rect.width,
      height: rect.height,
    }
  }, [])

  useEffect(() => {
    let cancelled = false
    const frame = requestAnimationFrame(async () => {
      const bounds = getBounds()
      if (!bounds) return
      try {
        await invoke('open_agent_ui_webview', {
          agentId: tab.agentId,
          url,
          ...bounds,
        })
        openedRef.current = true
        if (cancelled || !activeRef.current) {
          invoke('update_agent_ui_webview', {
            agentId: tab.agentId,
            ...bounds,
            visible: false,
          }).catch(() => {})
        }
      } catch (error) {
        console.error('Failed to embed agent UI', error)
      }
    })
    return () => {
      cancelled = true
      cancelAnimationFrame(frame)
    }
  }, [getBounds, tab.agentId, url])

  useEffect(() => {
    activeRef.current = active
    const syncBounds = () => {
      const bounds = getBounds()
      if (!bounds || !openedRef.current) return
      invoke('update_agent_ui_webview', {
        agentId: tab.agentId,
        ...bounds,
        visible: active,
      }).catch(() => {})
    }

    const observer = new ResizeObserver(syncBounds)
    if (containerRef.current) observer.observe(containerRef.current)
    window.addEventListener('resize', syncBounds)
    requestAnimationFrame(syncBounds)

    return () => {
      observer.disconnect()
      window.removeEventListener('resize', syncBounds)
      const bounds = getBounds() ?? { x: 0, y: 0, width: 1, height: 1 }
      invoke('update_agent_ui_webview', {
        agentId: tab.agentId,
        ...bounds,
        visible: false,
      }).catch(() => {})
    }
  }, [active, getBounds, tab.agentId])

  // The native webview is re-parented into a separate window for UI
  // fullscreen. Closing it restores the webview on the Rust side, but no DOM
  // resize necessarily follows. Sync after the React layout has settled.
  useEffect(() => {
    let unlisten: (() => void) | undefined
    let firstFrame = 0
    let secondFrame = 0
    listen<string>('agent-ui-fullscreen-closed', ({ payload }) => {
      if (payload !== tab.agentId) return
      firstFrame = requestAnimationFrame(() => {
        secondFrame = requestAnimationFrame(() => {
          const bounds = getBounds()
          if (!bounds || !openedRef.current) return
          invoke('update_agent_ui_webview', {
            agentId: tab.agentId,
            ...bounds,
            visible: activeRef.current,
          }).catch(() => {})
        })
      })
    }).then((dispose) => { unlisten = dispose }).catch(() => {})

    return () => {
      if (firstFrame) cancelAnimationFrame(firstFrame)
      if (secondFrame) cancelAnimationFrame(secondFrame)
      unlisten?.()
    }
  }, [getBounds, tab.agentId])

  return <div ref={containerRef} className="h-full w-full bg-white dark:bg-gray-950" />
}
