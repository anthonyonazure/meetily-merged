'use client';

import React from 'react';

interface State {
  error: Error | null;
}

/**
 * Catches render-time exceptions so a single broken component cannot blank the whole
 * window.
 *
 * Without this, React unmounts the entire tree on an uncaught render error and the app
 * shows nothing at all — no message, no trace, and nothing in the Rust log either,
 * because the failure happened in the webview. That is indistinguishable from a hang.
 */
export class AppErrorBoundary extends React.Component<{ children: React.ReactNode }, State> {
  state: State = { error: null };

  static getDerivedStateFromError(error: Error): State {
    return { error };
  }

  componentDidCatch(error: Error, info: React.ErrorInfo) {
    console.error('[AppErrorBoundary] Uncaught render error:', error, info.componentStack);
  }

  render() {
    const { error } = this.state;
    if (!error) return this.props.children;

    return (
      <div className="flex h-screen w-screen items-center justify-center bg-white p-8">
        <div className="max-w-2xl">
          <h1 className="mb-2 text-lg font-semibold text-gray-900">Something went wrong</h1>
          <p className="mb-4 text-sm text-gray-600">
            The interface failed to render. The error is below.
          </p>
          <pre className="max-h-80 overflow-auto whitespace-pre-wrap rounded-md bg-gray-100 p-4 text-xs text-red-700">
            {error.message}
            {'\n\n'}
            {error.stack}
          </pre>
          <button
            onClick={() => this.setState({ error: null })}
            className="mt-4 rounded-md border border-gray-300 px-3 py-2 text-sm hover:bg-gray-50"
          >
            Try again
          </button>
        </div>
      </div>
    );
  }
}

export default AppErrorBoundary;
