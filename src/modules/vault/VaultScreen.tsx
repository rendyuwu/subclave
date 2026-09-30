import { useVaultStore } from "./store";
import { FirstRunScreen } from "./FirstRunScreen";
import { UnlockScreen } from "./UnlockScreen";
import { VaultWorkspace } from "./VaultWorkspace";

/**
 * The vault half of the main pane. Picks one surface from the status probe:
 * nothing until the first probe answers, first run when there is no vault file,
 * the unlock screen while locked, the three panes otherwise.
 */
export function VaultScreen() {
  const status = useVaultStore((s) => s.status);

  if (status === null) return null;
  if (!status.exists) return <FirstRunScreen />;
  if (status.locked) return <UnlockScreen />;
  return <VaultWorkspace />;
}
