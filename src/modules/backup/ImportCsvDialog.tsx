import { ChevronDown } from "lucide-react";
import { useEffect, useState } from "react";

import { Button } from "@/components/ui/button";
import { Checkbox } from "@/components/ui/checkbox";
import {
  Dialog,
  DialogClose,
  DialogContent,
  DialogDescription,
  DialogFooter,
  DialogHeader,
  DialogTitle,
} from "@/components/ui/dialog";
import {
  DropdownMenu,
  DropdownMenuContent,
  DropdownMenuItem,
  DropdownMenuTrigger,
} from "@/components/ui/dropdown-menu";
import { Spinner } from "@/components/ui/spinner";
import { toast } from "@/components/ui/toast";
import { describeVaultError } from "@/modules/vault/errors";

import {
  importApply,
  importCsvPreview,
  importDeleteCsv,
  type CsvFormat,
  type ImportPreview,
} from "./ipc";

const FORMAT_LABELS: Record<CsvFormat, string> = {
  auto: "Detect from the header",
  keepassxc: "KeePassXC",
  bitwarden: "Bitwarden",
  chrome: "Chrome",
  firefox: "Firefox",
};

const FORMATS = Object.keys(FORMAT_LABELS) as CsvFormat[];

/**
 * Preview a picked CSV file, tick the rows to import, apply, and offer to
 * delete the plaintext file afterwards. The rows stay in Rust; the preview
 * holds titles, hosts and usernames only.
 */
export function ImportCsvDialog({ path, onClose }: { path: string | null; onClose: () => void }) {
  const [format, setFormat] = useState<CsvFormat>("auto");
  const [preview, setPreview] = useState<ImportPreview | null>(null);
  const [selected, setSelected] = useState<Set<number>>(new Set());
  const [loading, setLoading] = useState(false);
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const [added, setAdded] = useState<number | null>(null);
  const [deleted, setDeleted] = useState(false);
  // Reset on each opening, during render so the first frame of a new opening
  // never shows the last one's rows. `shownPath` outlives the close so the
  // path line does not blank out while the dialog fades.
  const [openedPath, setOpenedPath] = useState<string | null>(null);
  const [shownPath, setShownPath] = useState("");
  if (path !== openedPath) {
    setOpenedPath(path);
    if (path !== null) {
      setShownPath(path);
      setFormat("auto");
      setPreview(null);
      setSelected(new Set());
      setBusy(false);
      setError(null);
      setAdded(null);
      setDeleted(false);
    }
  }

  // Opening and every format change re-read the file. A late answer for a
  // format the user already moved away from is dropped.
  useEffect(() => {
    if (path === null) return;
    let current = true;
    setLoading(true);
    setError(null);
    importCsvPreview(path, format)
      .then((next) => {
        if (!current) return;
        setPreview(next);
        setSelected(new Set(next.rows.filter((r) => r.problem === null).map((r) => r.row)));
      })
      .catch((e: unknown) => {
        if (!current) return;
        setPreview(null);
        setSelected(new Set());
        setError(describeVaultError(String(e)));
      })
      .finally(() => {
        if (current) setLoading(false);
      });
    return () => {
      current = false;
    };
  }, [path, format]);

  const toggle = (row: number, on: boolean) => {
    setSelected((prev) => {
      const next = new Set(prev);
      if (on) next.add(row);
      else next.delete(row);
      return next;
    });
  };

  const runImport = async () => {
    if (!preview) return;
    setBusy(true);
    setError(null);
    try {
      const result = await importApply(preview.handle, [...selected]);
      setAdded(result.added);
    } catch (e) {
      setError(describeVaultError(String(e)));
    } finally {
      setBusy(false);
    }
  };

  const runDelete = async () => {
    if (!preview) return;
    setBusy(true);
    try {
      await importDeleteCsv(preview.handle);
      setDeleted(true);
      toast("CSV file deleted.", { variant: "success" });
    } catch (e) {
      toast(describeVaultError(String(e)), { variant: "error" });
    } finally {
      setBusy(false);
    }
  };

  const count = selected.size;
  const canImport = preview !== null && !loading && !busy && error === null && count > 0;

  return (
    <Dialog
      open={path !== null}
      // Every close route (Escape, outside click, the X, Cancel) lands here,
      // so this is the one place that can refuse a close while a write runs.
      onOpenChange={(next) => {
        if (!next && !busy) onClose();
      }}
    >
      <DialogContent className="sm:max-w-lg" showCloseButton={!busy}>
        <DialogHeader>
          <DialogTitle>Import CSV</DialogTitle>
          <DialogDescription>
            Imports logins exported from KeePassXC, Bitwarden, Chrome or Firefox into a new group.
            Flagged rows start unticked.
          </DialogDescription>
        </DialogHeader>

        <p className="text-muted-foreground truncate font-mono text-[10.5px]" title={shownPath}>
          {shownPath}
        </p>

        {added !== null ? (
          <div className="flex flex-col gap-3">
            <p className="text-[12px]">
              Imported {added} {added === 1 ? "entry" : "entries"}. The CSV file still holds every
              password in plain text.
            </p>
            <Button
              variant="outline"
              size="sm"
              disabled={busy || deleted}
              onClick={() => void runDelete()}
              className="gap-1.5 self-start"
            >
              {busy ? <Spinner className="size-3" /> : null}
              Delete the CSV file
            </Button>
          </div>
        ) : (
          <div className="flex min-h-0 flex-col gap-3">
            <div className="flex items-center gap-3">
              <span className="text-muted-foreground text-[11px] font-medium">Format</span>
              {/* Disabled while a preview runs too: a format change then
                  would race two previews for the one staging slot. */}
              <DropdownMenu>
                <DropdownMenuTrigger asChild>
                  <Button
                    variant="outline"
                    disabled={busy || loading}
                    className="h-9 justify-between gap-2 px-2.5 text-[12px]"
                  >
                    <span>{FORMAT_LABELS[format]}</span>
                    <ChevronDown size={12} strokeWidth={2} className="opacity-70" />
                  </Button>
                </DropdownMenuTrigger>
                <DropdownMenuContent align="start" className="min-w-[200px]">
                  {FORMATS.map((f) => (
                    <DropdownMenuItem key={f} onSelect={() => setFormat(f)} className="text-[12px]">
                      {FORMAT_LABELS[f]}
                    </DropdownMenuItem>
                  ))}
                </DropdownMenuContent>
              </DropdownMenu>
              {format === "auto" && preview && !loading ? (
                <span className="text-muted-foreground text-[11px]">
                  Detected: {FORMAT_LABELS[preview.format]}
                </span>
              ) : null}
              {loading ? <Spinner className="size-3" /> : null}
            </div>

            {error ? <p className="text-destructive text-[11px]">{error}</p> : null}

            {preview && error === null ? (
              preview.rows.length === 0 ? (
                <p className="text-muted-foreground text-[12px]">This file has no rows.</p>
              ) : (
                <div className="flex min-h-0 flex-col gap-1.5">
                  <span className="text-muted-foreground text-[11px] tabular-nums">
                    {preview.rows.length} {preview.rows.length === 1 ? "row" : "rows"}, {count}{" "}
                    selected
                  </span>
                  <div className="border-border/60 max-h-80 overflow-y-auto rounded-lg border p-1">
                    {preview.rows.map((r) => (
                      <label
                        key={r.row}
                        className="hover:bg-muted/50 flex items-start gap-2 rounded-md px-2 py-1.5 text-[12px] [contain-intrinsic-size:auto_44px] [content-visibility:auto]"
                      >
                        <Checkbox
                          className="mt-0.5"
                          checked={selected.has(r.row)}
                          disabled={busy}
                          onCheckedChange={(checked) => toggle(r.row, checked === true)}
                        />
                        <span className="flex min-w-0 flex-1 flex-col">
                          <span className="truncate font-medium">{r.title || "(no title)"}</span>
                          <span className="text-muted-foreground truncate text-[11px]">
                            {[r.host, r.username].filter(Boolean).join(" · ")}
                          </span>
                          {r.problem ? (
                            <span className="text-destructive text-[11px]">{r.problem}</span>
                          ) : null}
                        </span>
                      </label>
                    ))}
                  </div>
                </div>
              )
            ) : null}
          </div>
        )}

        <DialogFooter>
          <DialogClose asChild>
            <Button variant="outline" size="sm" disabled={busy}>
              {added !== null ? "Close" : "Cancel"}
            </Button>
          </DialogClose>
          {added === null ? (
            <Button
              size="sm"
              disabled={!canImport}
              onClick={() => void runImport()}
              className="gap-1.5"
            >
              {busy ? <Spinner className="size-3" /> : null}
              Import {count}
            </Button>
          ) : null}
        </DialogFooter>
      </DialogContent>
    </Dialog>
  );
}
