import { AlertCircle, AudioLines, ChevronRight, Clipboard, Play, RotateCcw, Trash2 } from "lucide-react";
import { errorMessage, progressLabel, translator, type CopyKey } from "../i18n";
import { type AppLanguage, type BootstrapState, type FailedRecording } from "../types";
import { type StatusTone } from "../ui-types";
import { appIsReady, appStatus, formatShortcut } from "../lib/presentation";
import { HistoryRow, EmptyHistory } from "../components/HistoryList";

export function Dashboard({ data, language, isBusy, onOpenOnboarding, onTest, onRetry, onDeleteFailed, onCopy, onOpen, onRetranscribe }: {
  data: BootstrapState;
  language: AppLanguage;
  isBusy: boolean;
  onOpenOnboarding: () => void;
  onTest: () => void;
  onRetry: () => void;
  onDeleteFailed: () => void;
  onCopy: (text: string) => void;
  onOpen: (path: string) => void;
  onRetranscribe: (id: string) => void;
}) {
  const t = translator(language);
  const shortcut = formatShortcut(data.settings.dictationShortcut);
  const isReady = appIsReady(data);
  const status = appStatus(data, language);
  const isProcessing = data.recording.state === "starting" || data.recording.state === "transcribing";
  const processingStage = progressLabel(data.recording.progress.stage, language);
  return (
    <div className="page dashboard">
      <header className="page-header"><div><span className="eyebrow">AIDOO WHISPER LITE</span><h1>{t("dictation")}</h1><p>{t("tagline")}</p></div><StatusPill tone={status.tone} label={status.label} /></header>
      {!isReady && !data.failedRecording && (
        <section className="setup-banner" role="status"><AlertCircle /><div><strong>{t("notReady")}</strong><span>{t("onboardingIncomplete")}</span></div><button onClick={onOpenOnboarding}>{t("openOnboarding")}<ChevronRight /></button></section>
      )}
      <section className={`dictation-hero ${data.recording.state}`}>
        <div className="hero-glow" />
        <div className="shortcut-key"><span>{shortcut}</span><small>{t("holdLabel")}</small></div>
        <div className="hero-copy"><span className="eyebrow">{t("pushToTalk")}</span><h2>{data.recording.state === "starting" ? progressLabel("starting_microphone", language) : data.recording.state === "recording" ? t("recording") : data.recording.state === "transcribing" ? t("transcribing") : data.recording.state === "done" ? progressLabel("text_ready", language) : data.recording.state === "error" ? t("errorState") : t("holdShortcut", { shortcut })}</h2><p>{isProcessing ? processingStage : data.recording.state === "error" && data.recording.error ? errorMessage(data.recording.error, language) : t("autoPasteHelp")}</p>{isProcessing && <div className={`hero-progress ${data.recording.progress.determinate ? "" : "indeterminate"}`} role="progressbar" aria-label={processingStage} aria-valuemin={data.recording.progress.determinate ? 0 : undefined} aria-valuemax={data.recording.progress.determinate ? 100 : undefined} aria-valuenow={data.recording.progress.determinate ? data.recording.progress.percent : undefined}><i style={{ width: data.recording.progress.determinate ? `${data.recording.progress.percent}%` : "38%" }} /></div>}</div>
        <button className="hero-action" disabled={!isReady || (isBusy && data.recording.state !== "recording")} onClick={onTest}>{data.recording.state === "recording" ? <><AudioLines />{t("stopTest")}</> : <><Play />{t("startTest")}</>}</button>
      </section>
      {data.failedRecording && <FailedCard failed={data.failedRecording} language={language} disabled={isBusy} onRetry={onRetry} onDelete={onDeleteFailed} />}
      <section className="section-card">
        <header><div><h3>{t("recent")}</h3><p>{t("saveHistoryHelp")}</p></div><button className="text-button" onClick={() => data.history[0] && onCopy(data.history[0].text)} disabled={isBusy || !data.history[0]}><Clipboard />{t("copy")}</button></header>
        {data.history.length ? data.history.slice(0, 4).map((entry) => <HistoryRow key={entry.id} entry={entry} language={language} actionBusy={isBusy} onCopy={onCopy} onOpen={onOpen} onRetranscribe={onRetranscribe} />) : <EmptyHistory language={language} />}
      </section>
    </div>
  );
}
function FailedCard({ failed, language, disabled, onRetry, onDelete }: { failed: FailedRecording; language: AppLanguage; disabled: boolean; onRetry: () => void; onDelete: () => void }) {
  const t = translator(language);
  const canFinishLocally = Boolean(failed.completedText);
  const canRetry = failed.retryable || canFinishLocally;
  const title: CopyKey = canFinishLocally ? "completedRecoveryTitle" : failed.retryable ? "failedTitle" : "retainedTitle";
  const body: CopyKey = canFinishLocally ? "completedRecoveryBody" : failed.retryable ? "failedBody" : "retainedBody";
  return <section className="failed-card" role="status"><AlertCircle /><div><strong>{t(title)}</strong><span>{t(body)}</span><small>{errorMessage(failed.error, language)}</small></div>{canRetry && <button className="secondary-button" disabled={disabled} onClick={onRetry}><RotateCcw />{t(canFinishLocally ? "finishLocally" : "retry")}</button>}<button className="icon-button danger" disabled={disabled} onClick={onDelete} aria-label={t("delete")}><Trash2 /></button></section>;
}
function StatusPill({ tone, label }: { tone: StatusTone; label: string }) {
  return <span className={`status-pill ${tone}`}><i />{label}</span>;
}
