// Draws a group's or an entry's optional icon and colour. The stored ids are
// plain strings on the model, so this file is the only place that turns them
// into a lucide component or a colour class. Colours are the active theme's
// ANSI tokens, each written out as a full class so Tailwind's source scan
// emits it. Nothing here reads or writes the store.

import { ToggleButton, Field } from "@/modules/vault/editor/FormControls";
import type { EntryColor } from "@/modules/vault/types";
import { cn } from "@/lib/utils";
import {
  Cloud,
  Container,
  Database,
  Globe,
  Laptop,
  Monitor,
  Router,
  Server,
  Shield,
  Terminal,
  type LucideIcon,
} from "lucide-react";

const ENTRY_ICONS: Record<string, { label: string; Icon: LucideIcon }> = {
  server: { label: "Server", Icon: Server },
  database: { label: "Database", Icon: Database },
  globe: { label: "Web", Icon: Globe },
  cloud: { label: "Cloud", Icon: Cloud },
  monitor: { label: "Desktop", Icon: Monitor },
  laptop: { label: "Laptop", Icon: Laptop },
  terminal: { label: "Shell", Icon: Terminal },
  shield: { label: "Firewall", Icon: Shield },
  router: { label: "Network device", Icon: Router },
  container: { label: "Container", Icon: Container },
};

const ENTRY_COLORS: Record<EntryColor, { label: string; className: string }> = {
  red: { label: "Red", className: "text-[color:var(--subclave-ansi-red)]" },
  yellow: { label: "Yellow", className: "text-[color:var(--subclave-ansi-yellow)]" },
  green: { label: "Green", className: "text-[color:var(--subclave-ansi-green)]" },
  cyan: { label: "Cyan", className: "text-[color:var(--subclave-ansi-cyan)]" },
  blue: { label: "Blue", className: "text-[color:var(--subclave-ansi-blue)]" },
  magenta: { label: "Magenta", className: "text-[color:var(--subclave-ansi-magenta)]" },
};

export const ENTRY_ICON_IDS: string[] = Object.keys(ENTRY_ICONS);
export const ENTRY_COLOR_IDS: EntryColor[] = Object.keys(ENTRY_COLORS) as EntryColor[];

/** A stored icon id this build does not know reads as no icon, so an entry
 *  from a newer build still renders. */
export function entryIconId(icon: string | null): string | null {
  return icon !== null && Object.hasOwn(ENTRY_ICONS, icon) ? icon : null;
}

/** A stored colour id this build does not know reads as no colour. */
export function entryColorId(color: EntryColor | null): EntryColor | null {
  return color !== null && Object.hasOwn(ENTRY_COLORS, color) ? color : null;
}

/** The entry's or group's icon in its colour (muted without one), else a
 *  swatch of its colour, else nothing. Decorative: the name always sits
 *  beside it. */
export function EntryGlyph({
  icon,
  color,
  className,
}: {
  icon: string | null;
  color: EntryColor | null;
  className?: string;
}) {
  const iconId = entryIconId(icon);
  const colorId = entryColorId(color);
  const tint = colorId ? ENTRY_COLORS[colorId].className : "text-muted-foreground";
  if (iconId) {
    const { Icon } = ENTRY_ICONS[iconId];
    return (
      <Icon size={14} strokeWidth={1.75} aria-hidden className={cn("shrink-0", tint, className)} />
    );
  }
  if (colorId) {
    return <span aria-hidden className={cn("size-2.5 shrink-0 bg-current", tint, className)} />;
  }
  return null;
}

/** The editor's icon and colour rows. A stored id this build does not know
 *  leaves no button pressed, and the draft keeps it until the user picks. */
export function AppearancePicker({
  icon,
  color,
  onIconChange,
  onColorChange,
}: {
  icon: string | null;
  color: EntryColor | null;
  onIconChange: (icon: string | null) => void;
  onColorChange: (color: EntryColor | null) => void;
}) {
  return (
    <>
      <Field label="Icon (optional)">
        <div role="group" aria-label="Icon" className="flex flex-wrap gap-1">
          <ToggleButton active={icon === null} onClick={() => onIconChange(null)}>
            None
          </ToggleButton>
          {ENTRY_ICON_IDS.map((id) => {
            const { label, Icon } = ENTRY_ICONS[id];
            return (
              <ToggleButton
                key={id}
                active={icon === id}
                onClick={() => onIconChange(id)}
                title={label}
              >
                <Icon size={14} strokeWidth={1.75} aria-hidden />
                <span className="sr-only">{label}</span>
              </ToggleButton>
            );
          })}
        </div>
      </Field>
      <Field label="Color (optional)">
        <div role="group" aria-label="Color" className="flex flex-wrap gap-1">
          <ToggleButton active={color === null} onClick={() => onColorChange(null)}>
            None
          </ToggleButton>
          {ENTRY_COLOR_IDS.map((id) => {
            const { label, className } = ENTRY_COLORS[id];
            return (
              <ToggleButton key={id} active={color === id} onClick={() => onColorChange(id)}>
                <span aria-hidden className={cn("size-2.5 shrink-0 bg-current", className)} />
                {label}
              </ToggleButton>
            );
          })}
        </div>
      </Field>
    </>
  );
}
