import { Component, type ReactNode } from 'react'
import i18n from '../i18n'

interface Props {
  children: ReactNode
  fallback?: ReactNode
}

interface State {
  hasError: boolean
  error: Error | null
}

/** 简单错误边界：捕获子树渲染异常，避免整页白屏 */
export class ErrorBoundary extends Component<Props, State> {
  state: State = { hasError: false, error: null }

  static getDerivedStateFromError(error: Error): State {
    return { hasError: true, error }
  }

  componentDidCatch(error: Error, info: unknown) {
    console.error('[ErrorBoundary]', error, info)
  }

  render() {
    if (this.state.hasError) {
      return (
        this.props.fallback ?? (
          <div className="flex h-full flex-col items-center justify-center gap-2 p-6 text-center">
            <p className="text-sm font-medium text-red-600 dark:text-red-400">
              {this.state.error?.message ?? i18n.t('errorBoundary.renderError')}
            </p>
            <button
              onClick={() => this.setState({ hasError: false, error: null })}
              className="text-xs text-blue-600 hover:underline dark:text-blue-400"
            >
              {i18n.t('errorBoundary.retry')}
            </button>
          </div>
        )
      )
    }
    return this.props.children
  }
}
