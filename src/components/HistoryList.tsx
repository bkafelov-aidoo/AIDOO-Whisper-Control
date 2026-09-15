import { AudioLines, Clipboard, Clock3, FileAudio, FileText, History, RotateCcw, Trash2 } from "lucide-react";
import { translator } from "../i18n";
import { type AppLanguage, type TranscriptEntry } from "../types";
import { formatDuration, formatDate, modelLabel } from "../lib/presentation";

export function HistoryRow({ entry, language, expanded = false, actionBusy = false, onCopy, onOpen, onRetranscribe, onDelete }: { entry: TranscriptEntry; language: AppLanguage; expanded?: boolean; actionBusy?: boolean; onCopy: (text: string) => void; onOpen: (path: string) => void; onRetranscribe: (id: string) => void; onDelete?: () => void }) {
  const t = translator(language);
  return <article className={`history-row ${expanded ? "expanded" : ""}`}><div className="history-icon"><AudioLines /></div><div className="history-copy"><p>{entry.text}</p><span><Clock3 />{formatDate(entry.createdAt, language)} · {formatDuration(entry.durationSeconds)} · {modelLabel(entry.model, language)}</span></div><div className="history-actions"><button title={t("copy")} aria-label={t("copy")} disabled={actionBusy} onClick={() => onCopy(entry.text)}><Clipboard /></button>{entry.audioPath && <button title={t("retranscribe")} aria-label={t("retranscribe")} disabled={actionBusy} onClick={() => onRetranscribe(entry.id)}><RotateCcw /></button>}{entry.audioPath && <button title={t("openAudio")} aria-label={t("openAudio")} disabled={actionBusy} onClick={() => onOpen(entry.audioPath!)}><FileAudio /></button>}{entry.textPath && <button title={t("openText")} aria-label={t("openText")} disabled={actionBusy} onClick={() => onOpen(entry.textPath!)}><FileText /></button>}{onDelete && <button className="danger" title={t("delete")} aria-label={t("delete")} disabled={actionBusy} onClick={onDelete}><Trash2 /></button>}</div></article>;
}
export function EmptyHistory({ language }: { language: AppLanguage }) {
  const t = translator(language);
  return <div className="empty-state"><History /><strong>{t("emptyHistory")}</strong></div>;
}
