import { useEffect, useRef, useState, useCallback } from 'react'

interface UseResizableOptions {
  minW?: number
  maxW?: number
  minH?: number
  maxH?: number
  defaultW?: number
  defaultH?: number
}

export function useResizable(options: UseResizableOptions = {}) {
  const {
    minW = 200, maxW = 480, defaultW = 288,
    minH = 120, maxH = 800, defaultH = 380,
  } = options

  const [width, setWidth] = useState(defaultW)
  const [height, setHeight] = useState(defaultH)

  const colDragging = useRef(false)
  const colStartX = useRef(0)
  const colStartW = useRef(defaultW)
  const rowDragging = useRef(false)
  const rowStartY = useRef(0)
  const rowStartH = useRef(defaultH)

  const onColMouseDown = useCallback((e: React.MouseEvent) => {
    colDragging.current = true
    colStartX.current = e.clientX
    colStartW.current = width
    document.body.style.cursor = 'col-resize'
    document.body.style.userSelect = 'none'
  }, [width])

  const onRowMouseDown = useCallback((e: React.MouseEvent) => {
    e.preventDefault()
    rowDragging.current = true
    rowStartY.current = e.clientY
    rowStartH.current = height
    document.body.style.cursor = 'row-resize'
    document.body.style.userSelect = 'none'
  }, [height])

  useEffect(() => {
    const onMove = (e: MouseEvent) => {
      if (colDragging.current) {
        const d = e.clientX - colStartX.current
        setWidth(Math.min(maxW, Math.max(minW, colStartW.current + d)))
      }
      if (rowDragging.current) {
        const d = e.clientY - rowStartY.current
        setHeight(Math.min(maxH, Math.max(minH, rowStartH.current + d)))
      }
    }
    const onUp = () => {
      colDragging.current = false
      rowDragging.current = false
      document.body.style.cursor = ''
      document.body.style.userSelect = ''
    }
    window.addEventListener('mousemove', onMove)
    window.addEventListener('mouseup', onUp)
    return () => {
      window.removeEventListener('mousemove', onMove)
      window.removeEventListener('mouseup', onUp)
    }
  }, [minW, maxW, minH, maxH])

  return { width, height, onColMouseDown, onRowMouseDown }
}
