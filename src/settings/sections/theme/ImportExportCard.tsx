import { Button } from "@/components/ui/button";
import { Download } from "lucide-react";
import { SettingRow } from "../../components/SettingRow";
import { SettingsAccordion } from "../../components/SettingsAccordion";
import { UploadButton } from "../../components/UploadButton";

type Props = {
  onImportFromDialog: () => Promise<void>;
  onExport: () => Promise<void>;
  importError: string | null;
  importStatus: string | null;
};

/** The Import / Export accordion: `.subclave` theme file in and out. */
export function ImportExportCard({
  onImportFromDialog,
  onExport,
  importError,
  importStatus,
}: Props) {
  return (
    <SettingsAccordion title="Import / Export">
      <div className="flex flex-col gap-2">
        <SettingRow
          title=".subclave theme file"
          description="Share themes with teammates. Files contain all colors plus background settings (image data is excluded from the export)."
        >
          <div className="flex items-center gap-2">
            <UploadButton onClick={() => void onImportFromDialog()}>Import</UploadButton>
            <Button
              variant="outline"
              size="sm"
              className="h-8 px-2 text-[11px]"
              onClick={() => void onExport()}
            >
              <Download size={12} strokeWidth={1.75} />
              Export
            </Button>
          </div>
        </SettingRow>
        {importError ? <span className="text-destructive text-[10.5px]">{importError}</span> : null}
        {importStatus ? (
          <span className="text-muted-foreground text-[10.5px]">{importStatus}</span>
        ) : null}
      </div>
    </SettingsAccordion>
  );
}
