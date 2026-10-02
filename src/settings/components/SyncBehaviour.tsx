import { Switch } from "@/components/ui/switch";
import type { SyncConfig } from "@/modules/sync/types";
import { Label } from "./Label";
import { SettingRow } from "./SettingRow";

/**
 * The Behaviour block of a sync form: the conditional-write switch and the note
 * on what its off position costs, or, on WebDAV, which has no conditional write
 * to offer, that cost stated with no switch.
 *
 * SHARED BY BOTH FORMS that configure a provider, Settings > Sync and the
 * first-run join screen. A join stores the configuration the device then runs
 * on, so a join form without this switch left every joined S3 device on
 * unconditional writes until somebody opened Settings.
 */
export function SyncBehaviour({
  config,
  onChange,
}: {
  config: SyncConfig;
  onChange: (config: SyncConfig) => void;
}) {
  // WebDAV has no conditional write to offer, so this block states that as a
  // cost instead of offering a switch for it.
  const webdav = config.provider === "webdav";

  return (
    <div className="flex flex-col gap-2">
      <Label>Behaviour</Label>
      {/* TWO WHOLE RENDERINGS rather than one with the toggle inside it. On a
          provider with no conditional write to offer there is no switch, and
          the note cannot then say "the setting above, which you chose" about
          a control that is not on the screen. */}
      {webdav ? (
        // A switch the user could move with no effect would be worse than no
        // switch: it would read as a promise. WebDAV leaves the conditional
        // write to each server, so this build never asks one for it, and the
        // consequence is stated unconditionally because nothing about it is
        // the user's to change.
        <div
          role="status"
          className="border-border/60 bg-card flex flex-col gap-1 rounded-lg border px-3 py-2.5"
        >
          <span className="text-[11.5px] font-semibold">Note: conditional writes are off</span>
          <span className="text-muted-foreground text-[10.5px] leading-relaxed">
            Two devices that write the same record at the same moment can leave only one of the two
            writes on the remote, and the other is lost without an error. WebDAV does not guarantee
            a server can refuse a write that would do that, so this app never asks one to, and there
            is nothing here to turn on. This is what this provider costs, not a setting you got
            wrong.
          </span>
        </div>
      ) : (
        <>
          <SettingRow
            title="Endpoint honours conditional writes"
            description="Turn this on only if you know your storage supports a write that fails when the object changed underneath it. This app does not test for it."
          >
            <Switch
              checked={config.cas}
              onCheckedChange={(v) => onChange({ ...config, cas: v })}
              aria-label="Endpoint honours conditional writes"
            />
          </SettingRow>
          {!config.cas ? (
            // Worded as a consequence of the SETTING, not as a finding.
            // Nothing in the app probes the endpoint, so a label claiming it
            // detected anything would be a claim no code backs.
            <div
              role="status"
              className="border-border/60 bg-card flex flex-col gap-1 rounded-lg border px-3 py-2.5"
            >
              <span className="text-[11.5px] font-semibold">Note: conditional writes are off</span>
              <span className="text-muted-foreground text-[10.5px] leading-relaxed">
                With this off, two devices that write the same record at the same moment can leave
                only one of the two writes on the remote, and the other is lost without an error. It
                is also what protects the sync folder itself: two devices creating it at the same
                moment both report success, and the second one's folder replaces the first one's,
                taking every record sealed under the key it held. Turning the switch on is what lets
                this app notice that refusal instead of writing over it. This app does not check
                what your endpoint supports; either setting is safe when only one device writes at a
                time.
              </span>
            </div>
          ) : null}
        </>
      )}
    </div>
  );
}
