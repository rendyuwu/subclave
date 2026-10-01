import type { LoginSummary } from "../lib/protocol";
import { OUTLINE_BUTTON, PRIMARY_BUTTON, SECTION_TITLE } from "./ui";

type Props = {
  entries: LoginSummary[];
  onGenerate: (entryId: string | null) => void;
};

export function PopupGenerate({ entries, onGenerate }: Props) {
  return (
    <div className="flex flex-col gap-1 border-t border-border pt-1">
      <p className={SECTION_TITLE}>Generate a password</p>
      {entries.map((entry) => (
        <button
          key={entry.id}
          type="button"
          className={OUTLINE_BUTTON}
          onClick={() => onGenerate(entry.id)}
        >
          Update {entry.title}
        </button>
      ))}
      <button type="button" className={PRIMARY_BUTTON} onClick={() => onGenerate(null)}>
        {entries.length > 0 ? "New entry" : "Generate for this site"}
      </button>
    </div>
  );
}
