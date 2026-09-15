import { translator } from "../i18n";
import { type AppLanguage, type TranscriptEntry } from "../types";
import { HistoryRow, EmptyHistory } from "../components/HistoryList";

export function HistoryPage({ history, language, isBusy, onCopy, onOpen, onRetranscribe, onDelete }: { history: TranscriptEntry[]; language: AppLanguage; isBusy: boolean; onCopy: (text: string) => void; onOpen: (path: string) => void; onRetranscribe: (id: string) => void; onDelete: (entry: TranscriptEntry) => void }) {
  const t = translator(language);
  return <div className="page"><header className="page-header"><div><span className="eyebrow">{t("localHistory")}</span><h1>{t("history")}</h1><p>{t("saveHistoryHelp")}</p></div><span className="count-pill">{history.length} / 10</span></header><section className="section-card history-card">{history.length ? history.map((entry) => <HistoryRow key={entry.id} entry={entry} language={language} expanded actionBusy={isBusy} onCopy={onCopy} onOpen={onOpen} onRetranscribe={onRetranscribe} onDelete={() => onDelete(entry)} />) : <EmptyHistory language={language} />}</section></div>;
}
