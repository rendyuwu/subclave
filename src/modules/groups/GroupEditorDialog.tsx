// The group create / rename / move dialog. The tree opens it, so the request
// carries the group to edit (null to create) and the parent to nest under.

import { AppearancePicker } from "./AppearancePicker";
import { GroupPicker } from "./GroupPicker";
import { ROOT_ID, descendantIds } from "./groupTree";
import { Button } from "@/components/ui/button";
import {
  Dialog,
  DialogContent,
  DialogDescription,
  DialogFooter,
  DialogHeader,
  DialogTitle,
} from "@/components/ui/dialog";
import { Input } from "@/components/ui/input";
import { Spinner } from "@/components/ui/spinner";
import { toast } from "@/components/ui/toast";
import { describeVaultError } from "@/modules/vault/errors";
import { vaultGroupUpsert } from "@/modules/vault/ipc";
import { useVaultStore } from "@/modules/vault/store";
import type { EntryColor, Group } from "@/modules/vault/types";
import { useEffect, useState } from "react";

export function GroupEditorDialog({
  request,
  onClose,
}: {
  request: { group: Group | null; parentId: string | null } | null;
  onClose: () => void;
}) {
  const [name, setName] = useState("");
  const [parentId, setParentId] = useState(ROOT_ID);
  const [icon, setIcon] = useState<string | null>(null);
  const [color, setColor] = useState<EntryColor | null>(null);
  const [busy, setBusy] = useState(false);
  const groups = useVaultStore((s) => s.groups);

  useEffect(() => {
    if (!request) return;
    setName(request.group?.name ?? "");
    setParentId(request.parentId ?? ROOT_ID);
    setIcon(request.group?.icon ?? null);
    setColor(request.group?.color ?? null);
  }, [request]);

  const save = async () => {
    const trimmed = name.trim();
    if (trimmed.length === 0 || busy) return;
    setBusy(true);
    try {
      // `parentId` is always an explicit id, "root" included: omitting it would
      // let Rust keep the group's current parent and silently drop a move.
      await vaultGroupUpsert({
        id: request?.group?.id ?? null,
        parentId,
        name: trimmed,
        icon,
        color,
      });
      await useVaultStore.getState().refresh();
      onClose();
    } catch (error) {
      toast(describeVaultError(String(error)), { variant: "error" });
    } finally {
      setBusy(false);
    }
  };

  return (
    <Dialog open={request !== null} onOpenChange={(open) => !open && onClose()}>
      <DialogContent>
        <DialogHeader>
          <DialogTitle>{request?.group ? "Edit group" : "New group"}</DialogTitle>
          <DialogDescription>
            {request?.group
              ? "Rename the group, change its parent or its appearance."
              : "Groups hold entries. Pick a parent to nest it."}
          </DialogDescription>
        </DialogHeader>
        <div className="flex flex-col gap-4">
          <label className="flex flex-col gap-1.5">
            <span className="text-muted-foreground text-[11px]">Name</span>
            <Input
              value={name}
              onChange={(event) => setName(event.target.value)}
              autoFocus
              onKeyDown={(event) => {
                if (event.key === "Enter") void save();
              }}
            />
          </label>
          <GroupPicker
            value={parentId}
            onChange={setParentId}
            exclude={request?.group ? [...descendantIds(request.group.id, groups)] : undefined}
            label="Parent"
          />
          <AppearancePicker
            icon={icon}
            color={color}
            onIconChange={setIcon}
            onColorChange={setColor}
          />
        </div>
        <DialogFooter>
          <Button variant="outline" onClick={onClose} disabled={busy}>
            Cancel
          </Button>
          <Button onClick={() => void save()} disabled={busy || name.trim().length === 0}>
            {busy ? <Spinner /> : null}
            {request?.group ? "Save" : "Create"}
          </Button>
        </DialogFooter>
      </DialogContent>
    </Dialog>
  );
}
