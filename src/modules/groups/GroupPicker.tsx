// The group choice used by the entry editor and the group editor: one option
// per group, labelled by its full path. Trash is never offered (Rust refuses a
// draft that names it) and neither is anything the caller excludes.

import { Combobox, type ComboboxOption } from "@/modules/vault/editor/Combobox";
import { ROOT_ID, TRASH_ID, groupPath } from "./groupTreeModel";
import { useVaultStore } from "@/modules/vault/store";

export function GroupPicker({
  value,
  onChange,
  exclude,
  label,
}: {
  value: string;
  onChange: (groupId: string) => void;
  exclude?: string[];
  label?: string;
}) {
  const groups = useVaultStore((s) => s.groups);
  const byId = new Map(groups.map((group) => [group.id, group]));
  const excluded = new Set(exclude ?? []);
  const options: ComboboxOption[] = [{ value: ROOT_ID, label: "Root", search: "root" }];
  for (const group of groups) {
    if (group.id === ROOT_ID || group.id === TRASH_ID || excluded.has(group.id)) continue;
    const path = groupPath(group.id, byId);
    options.push({ value: group.id, label: path, search: `${path} ${group.id}` });
  }
  options.sort((a, b) => a.label.localeCompare(b.label));

  return (
    <>
      {label ? <span className="text-muted-foreground text-[11px]">{label}</span> : null}
      <Combobox
        options={options}
        value={value}
        onChange={onChange}
        searchPlaceholder="Find a group"
        emptyLabel="No groups"
      />
    </>
  );
}
