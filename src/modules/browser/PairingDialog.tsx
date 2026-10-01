import {
  AlertDialog,
  AlertDialogAction,
  AlertDialogCancel,
  AlertDialogContent,
  AlertDialogDescription,
  AlertDialogFooter,
  AlertDialogHeader,
  AlertDialogTitle,
} from "@/components/ui/alert-dialog";
import { IPC_EVENTS } from "@/lib/ipc";
import { listen } from "@tauri-apps/api/event";
import { useEffect, useRef, useState } from "react";
import { browserPairingRespond } from "./ipc";

type PairingRequest = {
  requestId: string;
  browser: string;
  profileName: string;
  code: string;
};

/**
 * The pairing prompt an extension's `associate` raises. Rust reveals the main
 * window before emitting `IPC_EVENTS.PAIRING_REQUEST`, so this dialog is
 * actually seen even when `closeToTray` left the window hidden. Closing it any
 * way other than Allow (Escape, the X, Deny) answers `pairing-denied`.
 */
export function PairingDialog() {
  const [request, setRequest] = useState<PairingRequest | null>(null);
  // A close is followed by `onOpenChange(false)`, so one click can reach
  // `respond` twice; the id guard answers Rust once.
  const answered = useRef<string | null>(null);

  useEffect(() => {
    const unlisten = listen<PairingRequest>(IPC_EVENTS.PAIRING_REQUEST, (e) => {
      answered.current = null;
      setRequest(e.payload);
    });
    return () => {
      void unlisten.then((un) => un());
    };
  }, []);

  const respond = (accept: boolean) => {
    const current = request;
    setRequest(null);
    if (!current || answered.current === current.requestId) return;
    answered.current = current.requestId;
    void browserPairingRespond(current.requestId, accept).catch(() => undefined);
  };

  return (
    <AlertDialog
      open={request !== null}
      onOpenChange={(open) => {
        if (!open) respond(false);
      }}
    >
      <AlertDialogContent>
        <AlertDialogHeader>
          <AlertDialogTitle>Pair with Subclave?</AlertDialogTitle>
          <AlertDialogDescription>
            {request ? `${request.browser} (${request.profileName})` : "A browser"} wants to connect
            to Subclave. Compare the code below with the one the extension is showing.
          </AlertDialogDescription>
        </AlertDialogHeader>
        <div aria-live="polite" className="flex flex-col items-center gap-1 py-1">
          <span className="text-muted-foreground text-[10.5px]">Pairing code</span>
          <span className="font-mono text-2xl tracking-[0.4em]">{request?.code ?? ""}</span>
        </div>
        <AlertDialogFooter>
          <AlertDialogCancel onClick={() => respond(false)}>Deny</AlertDialogCancel>
          <AlertDialogAction onClick={() => respond(true)}>Allow</AlertDialogAction>
        </AlertDialogFooter>
      </AlertDialogContent>
    </AlertDialog>
  );
}
