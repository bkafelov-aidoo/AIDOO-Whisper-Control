import { Trash2, X } from "lucide-react";
import { translator } from "../i18n";
import { type AppLanguage, type TranscriptEntry } from "../types";
import { useDialogFocus } from "../hooks/useDialogFocus";

export function DeleteDialog({ entry, language, onCancel, onDelete }: { entry: TranscriptEntry; language: AppLanguage; onCancel: () => void; onDelete: (deleteFiles: boolean) => void }) {
  const t = translator(language);
  const dialogRef = useDialogFocus(onCancel);
  return <div className="modal-backdrop small"><section ref={dialogRef} className="confirm-dialog" role="alertdialog" aria-modal="true" aria-labelledby="delete-dialog-title" tabIndex={-1}><button className="close-button" aria-label={t("cancel")} onClick={onCancel}><X /></button><span className="danger-icon"><Trash2 /></span><h2 id="delete-dialog-title">{t("deleteQuestion")}</h2><p>{entry.text}</p><button className="secondary-button" onClick={() => onDelete(false)}>{t("historyOnly")}</button><button className="danger-button" onClick={() => onDelete(true)}>{t("historyAndFiles")}</button></section></div>;
}
