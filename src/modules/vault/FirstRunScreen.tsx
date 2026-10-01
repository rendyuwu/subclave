import { useState } from "react";
import { CreateVaultScreen } from "./CreateVaultScreen";
import { JoinSyncScreen } from "./JoinSyncScreen";

/**
 * First run: no vault file exists yet. Two cards, each swapping to its own
 * screen through the local-state pattern below.
 */
export function FirstRunScreen() {
  const [creating, setCreating] = useState(false);
  const [joining, setJoining] = useState(false);

  if (creating) return <CreateVaultScreen />;
  if (joining) {
    return (
      <JoinSyncScreen
        onCreateInstead={() => {
          setJoining(false);
          setCreating(true);
        }}
      />
    );
  }

  return (
    <div className="flex h-full items-center justify-center p-6">
      <div className="flex w-full max-w-sm flex-col gap-6">
        <div className="flex flex-col items-center gap-2 text-center">
          <img src="/icon.png" alt="" aria-hidden draggable={false} className="size-10" />
          <h1 className="text-xl font-semibold">Subclave</h1>
          <p className="text-muted-foreground text-sm">Your passwords, on your device.</p>
        </div>

        <div className="flex flex-col gap-2">
          <button
            type="button"
            onClick={() => setCreating(true)}
            className="border-border/60 bg-card hover:bg-muted/50 flex cursor-pointer flex-col items-start gap-0.5 rounded-lg border px-3 py-2.5 text-left transition-colors"
          >
            <span className="text-sm font-medium">Create a new vault</span>
            <span className="text-muted-foreground text-xs">
              Start with an empty vault protected by a master password.
            </span>
          </button>
          <button
            type="button"
            onClick={() => setJoining(true)}
            className="border-border/60 bg-card hover:bg-muted/50 flex cursor-pointer flex-col items-start gap-0.5 rounded-lg border px-3 py-2.5 text-left transition-colors"
          >
            <span className="text-sm font-medium">Join a synced vault</span>
            <span className="text-muted-foreground text-xs">
              Open the vault another device already syncs, using the same storage and passphrase.
            </span>
          </button>
        </div>
      </div>
    </div>
  );
}
