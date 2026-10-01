import { bootWindow } from "@/lib/boot";
import "@fontsource/jetbrains-mono/400.css";
import "@fontsource/jetbrains-mono/700.css";

import ReactDOM from "react-dom/client";
import App from "./app/App";
import { ErrorBoundary } from "./components/ErrorBoundary";
import { applyAppOpacityFastPath } from "@/modules/settings/appOpacity";
import { installFocusRestore } from "./lib/focusRestore";

const root = bootWindow({ rootId: "root" });

// Whole-app glass: fade the canvas toward the desktop on first paint so there
// is no opaque flash before hydration re-applies the stored value. Main window
// only - the settings window stays solid.
applyAppOpacityFastPath();

// Alt-Tab can leave the webview with focus on <body>, stranding the caret.
// Put it back where the user left it.
installFocusRestore();

ReactDOM.createRoot(root).render(
  <ErrorBoundary
    fallback={(error, reset) => (
      <div className="bg-background text-foreground flex h-screen w-screen flex-col items-center justify-center gap-3 p-6">
        <span className="text-sm font-semibold">Subclave hit an unexpected error.</span>
        <pre className="text-muted-foreground max-h-48 max-w-xl overflow-auto font-mono text-[11px] whitespace-pre-wrap">
          {error.message}
        </pre>
        <div className="flex gap-2">
          <button
            type="button"
            onClick={reset}
            className="border-border hover:bg-accent rounded-md border px-3 py-1.5 text-xs"
          >
            Try again
          </button>
          <button
            type="button"
            onClick={() => window.location.reload()}
            className="border-border hover:bg-accent rounded-md border px-3 py-1.5 text-xs"
          >
            Reload
          </button>
        </div>
      </div>
    )}
  >
    <App />
  </ErrorBoundary>,
);
