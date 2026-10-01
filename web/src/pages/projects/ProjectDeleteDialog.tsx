import { useEffect, useState } from "react";
import { UiButton, UiDialog, UiInput } from "@/components/ui";
import type { Translator } from "@/lib/i18n";

/** Confirms a permanent project deletion by asking for the project name. */
export function ProjectDeleteDialog({
  open,
  projectName,
  pending,
  error,
  onCancel,
  onConfirm,
  t
}: {
  open: boolean;
  projectName: string;
  pending: boolean;
  error: string | null;
  onCancel: () => void;
  onConfirm: () => Promise<void>;
  t: Translator;
}) {
  const [confirmation, setConfirmation] = useState("");
  const confirmed = confirmation.trim() === projectName;

  useEffect(() => {
    if (!open) setConfirmation("");
  }, [open]);

  return (
    <UiDialog
      open={open}
      title={t("projects.deleteDialogTitle")}
      description={t("projects.deleteDialogHint", { name: projectName })}
      onClose={onCancel}
      actions={
        <>
          <UiButton onClick={onCancel}>{t("common.cancel")}</UiButton>
          <UiButton
            variant="danger"
            disabled={!confirmed || pending}
            onClick={() => {
              if (confirmed && !pending) void onConfirm();
            }}
          >
            {pending ? t("projects.deleting") : t("projects.deleteAction")}
          </UiButton>
        </>
      }
    >
      <UiInput
        label={t("projects.deleteConfirmLabel", { name: projectName })}
        value={confirmation}
        autoComplete="off"
        spellCheck={false}
        onChange={(event) => setConfirmation(event.target.value)}
        error={error ?? undefined}
      />
    </UiDialog>
  );
}
