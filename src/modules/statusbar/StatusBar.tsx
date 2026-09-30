import { UpdaterPill } from "@/modules/updater";

/** One status-bar group. The hairline that separates it from the group before
 *  it is drawn by `.sb-group` in globals.css, which hides an empty group and
 *  only gives a group its divider when a non-empty one precedes it. */
function Group({ children }: { children: React.ReactNode }) {
  return <div className="sb-group flex shrink-0 items-center gap-1.5">{children}</div>;
}

export function StatusBar() {
  return (
    <footer className="border-border/60 bg-card/60 flex h-8 shrink-0 items-center justify-between gap-2 border-t px-3 text-[11px]">
      <div className="min-w-0 flex-1" />
      <Group>
        <UpdaterPill />
      </Group>
    </footer>
  );
}
