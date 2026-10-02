import { Button } from "@/components/ui/button";
import { BrandIcon } from "@/components/BrandIcon";
import { GITHUB_REPO, useUpdater } from "@/modules/updater/lib/useUpdater";
import { UpdaterDialog } from "@/modules/updater/components/UpdaterDialog";
import { updaterIcon, updaterLabel } from "@/modules/updater/components/UpdaterPill";
import { getName, getVersion } from "@tauri-apps/api/app";
import { openUrl } from "@tauri-apps/plugin-opener";
import { arch, platform } from "@tauri-apps/plugin-os";
import { useEffect, useState } from "react";
import { SectionHeader } from "../components/SectionHeader";
import { SettingsCard } from "../components/SettingsCard";

const REPO_URL = `https://github.com/${GITHUB_REPO}`;
const UPSTREAM_URL = "https://github.com/rendyuwu/tervia";

const PLATFORM_LABEL: Record<string, string> = {
  macos: "macOS",
  windows: "Windows",
  linux: "Linux",
  ios: "iOS",
  android: "Android",
  freebsd: "FreeBSD",
};

/** Synchronous platform/arch label. Returns "" off-Tauri (the calls throw). */
function initialBuildLabel(): string {
  try {
    const p = platform();
    const a = arch();
    const platformLabel = PLATFORM_LABEL[p] ?? p;
    return `${platformLabel} · ${a}`;
  } catch {
    return "";
  }
}

export function AboutSection() {
  const [version, setVersion] = useState("");
  const [name, setName] = useState("Subclave");
  const [build] = useState(initialBuildLabel);
  const [updateOpen, setUpdateOpen] = useState(false);
  // Click-only here: the main window's pill owns the 8s/6h sweeps.
  const updater = useUpdater({ autoCheck: false });
  const UpdateIcon = updaterIcon(updater.state);

  useEffect(() => {
    void getVersion().then(setVersion);
    void getName().then(setName);
  }, []);

  return (
    <div className="flex flex-col gap-6">
      <SectionHeader
        title="About"
        description="Version, build details, updates, and project links."
      />

      <div className="border-border/60 bg-card flex items-center gap-4 rounded-xl border p-5">
        <img src="/icon.png" alt="" className="size-12" draggable={false} />
        <div className="flex min-w-0 flex-col">
          <span className="text-[15px] font-semibold tracking-tight">{name}</span>
          <span className="text-muted-foreground text-[11px]">A local-first password manager.</span>
          <span className="text-muted-foreground mt-1 font-mono text-[11px]">
            v{version || "-"}
          </span>
        </div>
      </div>

      <SettingsCard
        title="Build details"
        description="Platform, bundle id, license, and source repositories."
      >
        <dl className="grid grid-cols-[110px_1fr] gap-y-2.5 text-[12px]">
          <dt className="text-muted-foreground">Build</dt>
          <dd className="font-mono text-[11.5px]">
            {build ? `${build} · v${version}` : `v${version}`}
          </dd>

          <dt className="text-muted-foreground">Bundle ID</dt>
          <dd className="font-mono text-[11.5px]">dev.rendy.subclave</dd>

          <dt className="text-muted-foreground">License</dt>
          <dd>Apache 2.0</dd>

          <dt className="text-muted-foreground">Source code</dt>
          <dd>
            <button
              type="button"
              onClick={() => void openUrl(REPO_URL)}
              className="hover:text-foreground inline-flex cursor-pointer items-center gap-1.5 rounded-md text-[12px] underline-offset-2 hover:underline"
            >
              <BrandIcon size={12} />
              rendyuwu/subclave
            </button>
          </dd>

          <dt className="text-muted-foreground">Built on</dt>
          <dd>
            <button
              type="button"
              onClick={() => void openUrl(UPSTREAM_URL)}
              className="hover:text-foreground inline-flex cursor-pointer items-center gap-1.5 rounded-md text-[12px] underline-offset-2 hover:underline"
            >
              <BrandIcon size={12} />
              rendyuwu/tervia
            </button>
          </dd>
        </dl>
      </SettingsCard>

      <div className="flex flex-wrap gap-2">
        {/* Same button copy as the status-bar UpdaterPill and the same shared
            dialog, but the check is click-only: opening Settings must not hit
            GitHub, so this button starts the check the way the old About-only
            state machine did. A check already in flight is left alone. */}
        <Button
          size="sm"
          onClick={() => {
            const k = updater.state.kind;
            if (k !== "checking" && k !== "downloading") {
              void updater.checkForUpdate();
            }
            setUpdateOpen(true);
          }}
          className="gap-1.5"
        >
          <UpdateIcon size={12} strokeWidth={1.75} />
          {updaterLabel(updater.state)}
        </Button>
        <Button
          variant="outline"
          size="sm"
          onClick={() => void openUrl(REPO_URL)}
          className="gap-1.5"
        >
          <BrandIcon size={12} />
          View on GitHub
        </Button>
        <Button variant="outline" size="sm" onClick={() => void openUrl(`${REPO_URL}/issues/new`)}>
          Report an issue
        </Button>
      </div>

      <UpdaterDialog
        open={updateOpen}
        onOpenChange={setUpdateOpen}
        state={updater.state}
        onInstall={() => void updater.downloadAndInstall()}
        onRelaunch={() => void updater.relaunchApp()}
        onRetry={() => void updater.checkForUpdate()}
      />
    </div>
  );
}
