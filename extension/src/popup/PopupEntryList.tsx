import type { LoginSummary } from "../lib/protocol";

type Props = {
  entries: LoginSummary[];
  activeIndex: number;
  onSelect: (entry: LoginSummary) => void;
  onActive: (index: number) => void;
};

export function PopupEntryList({ entries, activeIndex, onSelect, onActive }: Props) {
  return (
    <ul id="subclave-logins" role="listbox" aria-label="Logins" className="flex flex-col">
      {entries.map((entry, index) => (
        <li
          key={entry.id}
          id={`subclave-entry-${entry.id}`}
          role="option"
          aria-selected={index === activeIndex}
          className={`flex cursor-pointer flex-col px-2 py-1 ${
            index === activeIndex ? "bg-muted" : ""
          }`}
          onMouseEnter={() => onActive(index)}
          onMouseDown={(event) => event.preventDefault()}
          onClick={() => onSelect(entry)}
        >
          <span className="truncate text-foreground">{entry.title}</span>
          <span className="truncate text-muted-foreground">
            {entry.username} {entry.group}
          </span>
        </li>
      ))}
    </ul>
  );
}
