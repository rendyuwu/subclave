import { OUTLINE_BUTTON, PRIMARY_BUTTON } from "./ui";

type Props = {
  code: string | null;
  error: string | null;
  busy: boolean;
  onPair: () => void;
};

export function PopupPairing({ code, error, busy, onPair }: Props) {
  if (error) {
    return (
      <div className="flex flex-col gap-1 p-2">
        <p>{error}</p>
        <button type="button" className={PRIMARY_BUTTON} onClick={onPair}>
          Pair with Subclave
        </button>
      </div>
    );
  }
  if (code) {
    return (
      <div className="flex flex-col gap-1 p-2" aria-live="polite">
        <p>Waiting for Subclave</p>
        <p className="font-mono text-lg tracking-widest">{code}</p>
      </div>
    );
  }
  return (
    <div className="flex flex-col gap-1 p-2">
      <p>This browser is not paired with Subclave yet.</p>
      <button type="button" className={OUTLINE_BUTTON} disabled={busy} onClick={onPair}>
        Pair with Subclave
      </button>
    </div>
  );
}
