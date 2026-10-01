type Props = {
  checked: boolean;
  onChange: (value: boolean) => void;
};

export function PopupSettings({ checked, onChange }: Props) {
  return (
    <label className="flex items-center gap-2 border-t border-border px-2 py-2">
      <input
        type="checkbox"
        role="switch"
        aria-checked={checked}
        checked={checked}
        onChange={(event) => onChange(event.target.checked)}
      />
      <span>Show in login fields</span>
    </label>
  );
}
