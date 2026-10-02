import { bootWindow } from "@/lib/boot";
import ReactDOM from "react-dom/client";
import { ThemeProvider } from "@/modules/theme";
import { SettingsApp } from "./SettingsApp";

const root = bootWindow({ rootId: "settings-root" });

ReactDOM.createRoot(root).render(
  <ThemeProvider>
    <SettingsApp />
  </ThemeProvider>,
);

// Mono font is only used by a handful of inputs/labels - load it after the
// first paint so it doesn't block the initial render of the settings shell.
// Browser falls back to the CSS chain (SFMono/Menlo/monospace) until ready.
setTimeout(() => {
  void import("@fontsource/jetbrains-mono/400.css");
  void import("@fontsource/jetbrains-mono/700.css");
}, 0);
