import { Checkbox } from "@/components/ui/checkbox";
import { Input } from "@/components/ui/input";
import { Textarea } from "@/components/ui/textarea";
import { vaultEntryReveal } from "@/modules/vault/ipc";
import { AppearancePicker } from "@/modules/groups/AppearancePicker";
import { GroupPicker } from "@/modules/groups/GroupPicker";
import { CustomFieldsEditor } from "./CustomFieldsEditor";
import { Field } from "./FormControls";
import { GeneratorPopover } from "./GeneratorPopover";
import { SecretField } from "./SecretField";
import { TagsInput } from "./TagsInput";
import { TotpField } from "./TotpField";
import { UrlListField } from "./UrlListField";
import { dateInputToEpoch, epochToDateInput, type EditorDraft } from "./draft";

// The editor's scrollable row list: every field of one entry, plus the reveal
// and copy callbacks that read a stored secret back on demand. Presentational
// only, the draft and its patch come from the dialog.

export function EntryEditorFields({
  draft,
  patch,
  id,
  tagSuggestions,
  revealKey,
  onPasswordGenerated,
  reportCopy,
}: {
  draft: EditorDraft;
  patch: (part: Partial<EditorDraft>) => void;
  /** The stored entry's id, or null while creating. */
  id: string | null;
  tagSuggestions: string[];
  revealKey: number;
  onPasswordGenerated: () => void;
  reportCopy: (kind: string, storeField: string, id: string) => void;
}) {
  return (
    <div className="flex max-h-[62vh] flex-col gap-3 overflow-y-auto pr-1">
      <Field label="Title">
        <Input
          value={draft.title}
          onChange={(e) => patch({ title: e.target.value })}
          className="h-8"
        />
      </Field>
      <Field label="Username">
        <Input
          value={draft.username}
          spellCheck={false}
          autoComplete="off"
          onChange={(e) => patch({ username: e.target.value })}
          className="h-8"
        />
      </Field>
      <Field label="Password">
        <SecretField
          value={draft.password}
          ariaLabel="Password"
          resetKey={id ?? "new"}
          revealKey={revealKey}
          onChange={(password) => patch({ password, passwordTouched: true })}
          onReveal={id === null ? undefined : () => vaultEntryReveal(id, "password")}
          onCopy={id === null ? undefined : () => reportCopy("password", "password", id)}
        >
          <GeneratorPopover
            onUse={(password) => {
              patch({ password, passwordTouched: true });
              onPasswordGenerated();
            }}
          />
        </SecretField>
      </Field>
      <TotpField value={draft.totp} onChange={(totp) => patch({ totp, totpTouched: true })} />
      <UrlListField urls={draft.urls} onChange={(urls) => patch({ urls })} />
      <Field label="Notes">
        <Textarea
          value={draft.notes}
          onChange={(e) => patch({ notes: e.target.value })}
          className="min-h-20 text-[12px]"
        />
      </Field>
      <GroupPicker
        value={draft.groupId}
        onChange={(groupId) => patch({ groupId })}
        exclude={["trash"]}
        label="Group"
      />
      <CustomFieldsEditor
        fields={draft.customFields}
        onChange={(customFields) => patch({ customFields })}
        onReveal={id === null ? undefined : (name) => vaultEntryReveal(id, `custom:${name}`)}
        onCopy={id === null ? undefined : (name) => reportCopy(name, `custom:${name}`, id)}
      />
      <TagsInput
        tags={draft.tags}
        onChange={(tags) => patch({ tags: [...tags] })}
        suggestions={tagSuggestions}
      />
      <AppearancePicker
        icon={draft.icon}
        color={draft.color}
        onIconChange={(icon) => patch({ icon })}
        onColorChange={(color) => patch({ color })}
      />
      <label className="flex items-center gap-2 text-[12px]">
        <Checkbox
          checked={draft.favorite}
          onCheckedChange={(checked) => patch({ favorite: checked === true })}
        />
        Favourite
      </label>
      <Field label="Expires (optional)">
        <Input
          type="date"
          value={epochToDateInput(draft.expiresAt)}
          onChange={(e) => patch({ expiresAt: dateInputToEpoch(e.target.value) })}
          className="h-8"
        />
      </Field>
    </div>
  );
}
