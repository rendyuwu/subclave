import { useRef } from "react";
import { Plus, X } from "lucide-react";

import { Button } from "@/components/ui/button";
import { Checkbox } from "@/components/ui/checkbox";
import { Input } from "@/components/ui/input";
import { Field } from "./FormControls";
import { SecretField } from "./SecretField";
import type { EditorCustomField } from "./draft";

// The editor's custom-field rows. A hidden field's value arrives masked and its
// name is read-only until the value has been revealed: Rust matches a kept
// value by name, so renaming one whose value this session never loaded would
// silently lose the edit.

export function CustomFieldsEditor({
  fields,
  onChange,
  onReveal,
  onCopy,
}: {
  fields: EditorCustomField[];
  onChange: (fields: EditorCustomField[]) => void;
  /** Fetch an existing field's stored value on reveal (`custom:<name>`). */
  onReveal?: (name: string) => Promise<string | null>;
  onCopy?: (name: string) => void;
}) {
  // SecretField keeps the revealed value in its own state, so a row keyed by
  // its array index would hand that state to whatever slides into the freed
  // slot: removing a revealed row would show its secret on the next one. Each
  // row gets an id when it appears, kept aligned with the array by the add and
  // remove paths below; the length check only covers a caller handing in an
  // array of a different size.
  const idsRef = useRef<number[]>([]);
  const nextIdRef = useRef(0);
  while (idsRef.current.length < fields.length) idsRef.current.push(nextIdRef.current++);
  if (idsRef.current.length > fields.length) idsRef.current.length = fields.length;

  function update(index: number, patch: Partial<EditorCustomField>): void {
    onChange(fields.map((field, i) => (i === index ? { ...field, ...patch } : field)));
  }

  function remove(index: number): void {
    idsRef.current.splice(index, 1);
    onChange(fields.filter((_, i) => i !== index));
  }

  function add(): void {
    idsRef.current.push(nextIdRef.current++);
    onChange([...fields, { name: "", value: "", hidden: false, valueLoaded: false }]);
  }

  return (
    <Field label="Custom fields">
      <div className="flex flex-col gap-2">
        {fields.map((field, index) => (
          <div
            key={idsRef.current[index]}
            className="border-border/60 flex flex-col gap-1.5 rounded-xl border p-2"
          >
            <div className="flex items-center gap-1.5">
              <Input
                aria-label={`Custom field ${index + 1} name`}
                value={field.name}
                readOnly={field.hidden && !field.valueLoaded}
                placeholder="Name"
                spellCheck={false}
                autoComplete="off"
                onChange={(e) => update(index, { name: e.target.value })}
                className="h-8 text-[12px]"
              />
              <Button
                type="button"
                variant="ghost"
                size="icon-sm"
                aria-label={`Remove custom field ${index + 1}`}
                onClick={() => remove(index)}
              >
                <X />
              </Button>
            </div>
            <SecretField
              value={field.value}
              ariaLabel={`Value for ${field.name || `custom field ${index + 1}`}`}
              onChange={(value) => update(index, { value, valueLoaded: true })}
              onReveal={
                !field.valueLoaded && field.name.trim() !== "" && onReveal
                  ? async () => {
                      const stored = await onReveal(field.name);
                      if (stored !== null) update(index, { value: stored, valueLoaded: true });
                      return stored;
                    }
                  : undefined
              }
              onCopy={onCopy ? () => onCopy(field.name) : undefined}
            />
            <label className="text-muted-foreground flex items-center gap-2 text-[11px]">
              <Checkbox
                checked={field.hidden}
                onCheckedChange={(checked) => update(index, { hidden: checked === true })}
              />
              Hidden
            </label>
          </div>
        ))}
        <Button type="button" variant="outline" size="sm" className="self-start" onClick={add}>
          <Plus />
          Add field
        </Button>
      </div>
    </Field>
  );
}
