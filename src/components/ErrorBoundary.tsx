import { Component, type ErrorInfo, type ReactNode } from "react";

type Props = {
  children: ReactNode;
  /** Rendered in place of the subtree when it throws. Receives the error and a
   *  reset callback that clears it. */
  fallback: (error: Error, reset: () => void) => ReactNode;
};

type State = { error: Error | null };

/**
 * React error boundary for the main window. A render-time throw in the wrapped
 * subtree is contained here instead of unmounting the whole root tree to a
 * blank screen.
 */
export class ErrorBoundary extends Component<Props, State> {
  state: State = { error: null };

  static getDerivedStateFromError(error: Error): Partial<State> {
    return { error };
  }

  componentDidCatch(error: Error, info: ErrorInfo): void {
    console.error("ErrorBoundary caught:", error, info.componentStack);
  }

  private reset = () => this.setState({ error: null });

  render() {
    const { error } = this.state;
    if (!error) return this.props.children;
    return this.props.fallback(error, this.reset);
  }
}
