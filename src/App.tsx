import { useCallback, useEffect, useRef, useState } from "react";
import { invoke } from "@tauri-apps/api/core";
import { listen } from "@tauri-apps/api/event";
import { open } from "@tauri-apps/plugin-dialog";
import { openUrl } from "@tauri-apps/plugin-opener";
import { disable, enable, isEnabled } from "@tauri-apps/plugin-autostart";
import {
  AlertCircle,
  AudioLines,
  Check,
  ChevronLeft,
  ChevronRight,
  CircleHelp,
  Clipboard,
  Clock3,
  ExternalLink,
  FileAudio,
  FileText,
  FolderOpen,
  History,
  KeyRound,
  Languages,
  LoaderCircle,
  Mic,
  Play,
  Power,
  RefreshCw,
  RotateCcw,
  Settings,
  ShieldCheck,
  Sparkles,
  Trash2,
  X,
} from "lucide-react";
import { errorMessage, progressLabel, resolveLanguage, translator, type CopyKey } from "./i18n";
import type {
  AppLanguage,
  AppSettings,
  BootstrapState,
  FailedRecording,
  MicrophoneProbe,
  RecordingSnapshot,
  ShortcutBinding,
  TranscriptEntry,
  TranscriptionCompleted,
} from "./types";

type Page = "dictation" | "history" | "settings";
type ToastTone = "success" | "warning" | "error";
type ToastHandler = (message: string, tone?: ToastTone) => void;

export default function App() {
  const [data, setData] = useState<BootstrapState | null>(null);
  const [bootstrapError, setBootstrapError] = useState<string | null>(null);
  const [page, setPage] = useState<Page>("dictation");
  const [showOnboarding, setShowOnboarding] = useState(false);
  const [toast, setToast] = useState<{ message: string; tone: ToastTone } | null>(null);
  const [deleteTarget, setDeleteTarget] = useState<TranscriptEntry | null>(null);
  const toastTimer = useRef<number | null>(null);
  const languageRef = useRef<AppLanguage>("bg");

  const showToast = useCallback<ToastHandler>((message, tone = "success") => {
    setToast({ message, tone });
    if (toastTimer.current) window.clearTimeout(toastTimer.current);
    toastTimer.current = window.setTimeout(() => setToast(null), 4200);
  }, []);

  const refresh = useCallback(async () => {
    try {
      const next = await invoke<BootstrapState>("bootstrap");
      setData((current) => current && settingsMatch(current.settings, next.settings) ? { ...next, settings: current.settings } : next);
      setBootstrapError(null);
      return next;
    } catch (reason) {
      setBootstrapError(String(reason).replace(/^Error:\s*/, ""));
      throw reason;
    }
  }, []);

  useEffect(() => {
    void refresh()
      .then((next) => setShowOnboarding(!next.settings.onboardingComplete))
      .catch(() => undefined);
    const unlisten: Array<() => void> = [];
    void listen<TranscriptionCompleted>("transcription:completed", ({ payload }) => {
      setData((current) => {
        if (!current || !payload.entry) return current;
        return { ...current, history: [payload.entry, ...current.history.filter((item) => item.id !== payload.entry!.id)].slice(0, 10) };
      });
    }).then((fn) => unlisten.push(fn));
    void listen<string>("toast", ({ payload }) => showToast(errorMessage(payload, languageRef.current), "warning")).then((fn) => unlisten.push(fn));
    void listen<FailedRecording | null>("failed-recording:changed", ({ payload }) => {
      setData((current) => current ? { ...current, failedRecording: payload } : current);
    }).then((fn) => unlisten.push(fn));
    void listen<string>("recording:state", ({ payload }) => {
      setData((current) => current ? { ...current, recording: { ...current.recording, state: payload as BootstrapState["recording"]["state"] } } : current);
    }).then((fn) => unlisten.push(fn));
    void listen<RecordingSnapshot>("recording:snapshot", ({ payload }) => {
      setData((current) => current ? { ...current, recording: payload } : current);
    }).then((fn) => unlisten.push(fn));
    void listen<Page>("navigate", ({ payload }) => setPage(payload)).then((fn) => unlisten.push(fn));
    return () => {
      unlisten.forEach((fn) => fn());
      if (toastTimer.current) window.clearTimeout(toastTimer.current);
    };
  }, [refresh, showToast]);

  useEffect(() => {
    const refreshOnFocus = () => { void refresh().catch(() => undefined); };
    window.addEventListener("focus", refreshOnFocus);
    return () => window.removeEventListener("focus", refreshOnFocus);
  }, [refresh]);

  const language = resolveLanguage(data?.settings.uiLanguage ?? "auto");
  languageRef.current = language;
  const t = translator(language);

  const persistSettings = useCallback(async (settings: AppSettings) => {
    const saved = await invoke<AppSettings>("update_settings", { settings });
    setData((current) => current ? { ...current, settings: saved } : current);
    return saved;
  }, []);

  if (!data) {
    return <main className="loading-screen"><img src="/app-icon.png" alt="" />{bootstrapError ? <><p>{t("loadFailed")}</p><small>{bootstrapError}</small><button className="primary-button" onClick={() => void refresh().catch(() => undefined)}><RefreshCw />{t("retryLoad")}</button></> : <LoaderCircle className="spin" />}</main>;
  }

  const shortcut = formatShortcut(data.settings.dictationShortcut);
  const isBusy = ["starting", "recording", "transcribing"].includes(data.recording.state);
  const isReady = appIsReady(data);

  const runTestDictation = async () => {
    try {
      if (data.recording.state === "recording") {
        await invoke("stop_and_transcribe");
      } else {
        await invoke("start_recording");
      }
    } catch (reason) {
      showToast(errorMessage(reason, language), "error");
    }
  };

  const retranscribe = async (id: string) => {
    try {
      await invoke("retranscribe_history_item", { id });
      await refresh();
    } catch (reason) {
      showToast(errorMessage(reason, language), "error");
    }
  };

  return (
    <div className="app-shell" aria-busy={isBusy}>
      <aside className="sidebar">
        <div className="brand">
          <img src="/app-icon.png" alt="" />
          <div><strong>AIDOO</strong><span>Whisper Lite</span></div>
        </div>
        <nav>
          <NavButton active={page === "dictation"} disabled={isBusy} icon={<Mic />} label={t("dictation")} onClick={() => setPage("dictation")} />
          <NavButton active={page === "history"} disabled={isBusy} icon={<History />} label={t("history")} badge={data.history.length || undefined} onClick={() => setPage("history")} />
          <NavButton active={page === "settings"} disabled={isBusy} icon={<Settings />} label={t("settings")} onClick={() => setPage("settings")} />
        </nav>
        <div className={`sidebar-status ${isReady ? "ready" : "attention"}`}>
          <i />
          <span>{data.failedRecording ? t("actionRequired") : isReady ? t("ready") : t("notReady")}</span>
          <kbd>{shortcut}</kbd>
        </div>
      </aside>

      <main className="content">
        {page === "dictation" && (
          <Dashboard
            data={data}
            language={language}
            isBusy={isBusy}
            onOpenOnboarding={() => setShowOnboarding(true)}
            onTest={runTestDictation}
            onRetry={async () => {
              try { await invoke("retry_failed_transcription"); await refresh(); }
              catch (reason) { showToast(errorMessage(reason, language), "error"); }
            }}
            onDeleteFailed={async () => {
              try { await invoke("delete_failed_recording"); await refresh(); showToast(t("deleted")); }
              catch (reason) { showToast(errorMessage(reason, language), "error"); }
            }}
            onCopy={async (text) => {
              try { await invoke("copy_text", { text }); showToast(t("copied")); }
              catch (reason) { showToast(errorMessage(reason, language), "error"); }
            }}
            onOpen={async (path) => {
              try { await invoke("open_local_path", { path, reveal: false }); }
              catch (reason) { showToast(errorMessage(reason, language), "error"); }
            }}
            onRetranscribe={retranscribe}
          />
        )}
        {page === "history" && (
          <HistoryPage
            history={data.history}
            language={language}
            isBusy={isBusy}
            onCopy={async (text) => {
              try { await invoke("copy_text", { text }); showToast(t("copied")); }
              catch (reason) { showToast(errorMessage(reason, language), "error"); }
            }}
            onOpen={async (path) => {
              try { await invoke("open_local_path", { path, reveal: false }); }
              catch (reason) { showToast(errorMessage(reason, language), "error"); }
            }}
            onRetranscribe={retranscribe}
            onDelete={setDeleteTarget}
          />
        )}
        {page === "settings" && (
          <SettingsPage
            data={data}
            language={language}
            isBusy={isBusy}
            onSave={async (settings) => {
              const previousLaunchAtLogin = data.settings.launchAtLogin;
              const launchAtLoginChanged = previousLaunchAtLogin !== settings.launchAtLogin;
              try {
                if (launchAtLoginChanged) {
                  if (settings.launchAtLogin) await enable(); else await disable();
                }
                try {
                  await persistSettings(settings);
                } catch (reason) {
                  if (launchAtLoginChanged) {
                    try {
                      if (previousLaunchAtLogin) await enable(); else await disable();
                    } catch { /* The original error is more useful to the user. */ }
                  }
                  throw reason;
                }
                showToast(t("saved"));
              } catch (reason) { showToast(errorMessage(reason, language), "error"); }
            }}
            onRefresh={refresh}
            onToast={showToast}
            onOpenOnboarding={() => setShowOnboarding(true)}
          />
        )}
      </main>

      {showOnboarding && (
        <Onboarding
          data={data}
          language={language}
          isBusy={isBusy}
          onData={setData}
          onPersist={persistSettings}
          onRefresh={refresh}
          onClose={() => setShowOnboarding(false)}
          onToast={showToast}
        />
      )}

      {deleteTarget && (
        <DeleteDialog
          entry={deleteTarget}
          language={language}
          onCancel={() => setDeleteTarget(null)}
          onDelete={async (deleteFiles) => {
            try {
              await invoke("delete_history_item", { id: deleteTarget.id, deleteFiles });
              setDeleteTarget(null);
              await refresh();
              showToast(t("deleted"));
            } catch (reason) {
              setDeleteTarget(null);
              await refresh().catch(() => undefined);
              showToast(errorMessage(reason, language), "error");
            }
          }}
        />
      )}

      {toast && <div className={`toast ${toast.tone}`} role={toast.tone === "error" ? "alert" : "status"} aria-live={toast.tone === "error" ? "assertive" : "polite"}>{toast.tone === "success" ? <Check /> : <AlertCircle />}{toast.message}</div>}
    </div>
  );
}

function NavButton({ active, disabled, icon, label, badge, onClick }: { active: boolean; disabled: boolean; icon: React.ReactNode; label: string; badge?: number; onClick: () => void }) {
  return <button className={active ? "active" : ""} aria-current={active ? "page" : undefined} disabled={disabled} onClick={onClick}>{icon}<span>{label}</span>{badge ? <em>{badge}</em> : null}</button>;
}

function Dashboard({ data, language, isBusy, onOpenOnboarding, onTest, onRetry, onDeleteFailed, onCopy, onOpen, onRetranscribe }: {
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
  return (
    <div className="page dashboard">
      <header className="page-header"><div><span className="eyebrow">AIDOO WHISPER LITE</span><h1>{t("dictation")}</h1><p>{t("tagline")}</p></div><StatusPill ready={isReady} label={data.failedRecording ? t("actionRequired") : isReady ? t("ready") : t("notReady")} /></header>
      {!isReady && !data.failedRecording && (
        <section className="setup-banner" role="status"><AlertCircle /><div><strong>{t("notReady")}</strong><span>{t("onboardingIncomplete")}</span></div><button onClick={onOpenOnboarding}>{t("openOnboarding")}<ChevronRight /></button></section>
      )}
      <section className={`dictation-hero ${data.recording.state}`}>
        <div className="hero-glow" />
        <div className="shortcut-key"><span>{shortcut}</span><small>{t("holdLabel")}</small></div>
        <div className="hero-copy"><span className="eyebrow">{t("pushToTalk")}</span><h2>{data.recording.state === "recording" ? t("recording") : data.recording.state === "transcribing" ? t("transcribing") : t("holdShortcut", { shortcut })}</h2><p>{data.recording.state === "transcribing" ? progressLabel(data.recording.progress.stage, language) : t("autoPasteHelp")}</p></div>
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

function HistoryPage({ history, language, isBusy, onCopy, onOpen, onRetranscribe, onDelete }: { history: TranscriptEntry[]; language: AppLanguage; isBusy: boolean; onCopy: (text: string) => void; onOpen: (path: string) => void; onRetranscribe: (id: string) => void; onDelete: (entry: TranscriptEntry) => void }) {
  const t = translator(language);
  return <div className="page"><header className="page-header"><div><span className="eyebrow">{t("localHistory")}</span><h1>{t("history")}</h1><p>{t("saveHistoryHelp")}</p></div><span className="count-pill">{history.length} / 10</span></header><section className="section-card history-card">{history.length ? history.map((entry) => <HistoryRow key={entry.id} entry={entry} language={language} expanded actionBusy={isBusy} onCopy={onCopy} onOpen={onOpen} onRetranscribe={onRetranscribe} onDelete={() => onDelete(entry)} />) : <EmptyHistory language={language} />}</section></div>;
}

function HistoryRow({ entry, language, expanded = false, actionBusy = false, onCopy, onOpen, onRetranscribe, onDelete }: { entry: TranscriptEntry; language: AppLanguage; expanded?: boolean; actionBusy?: boolean; onCopy: (text: string) => void; onOpen: (path: string) => void; onRetranscribe: (id: string) => void; onDelete?: () => void }) {
  const t = translator(language);
  return <article className={`history-row ${expanded ? "expanded" : ""}`}><div className="history-icon"><AudioLines /></div><div className="history-copy"><p>{entry.text}</p><span><Clock3 />{formatDate(entry.createdAt, language)} · {formatDuration(entry.durationSeconds)} · {modelLabel(entry.model, language)}</span></div><div className="history-actions"><button title={t("copy")} aria-label={t("copy")} disabled={actionBusy} onClick={() => onCopy(entry.text)}><Clipboard /></button>{entry.audioPath && <button title={t("retranscribe")} aria-label={t("retranscribe")} disabled={actionBusy} onClick={() => onRetranscribe(entry.id)}><RotateCcw /></button>}{entry.audioPath && <button title={t("openAudio")} aria-label={t("openAudio")} disabled={actionBusy} onClick={() => onOpen(entry.audioPath!)}><FileAudio /></button>}{entry.textPath && <button title={t("openText")} aria-label={t("openText")} disabled={actionBusy} onClick={() => onOpen(entry.textPath!)}><FileText /></button>}{onDelete && <button className="danger" title={t("delete")} aria-label={t("delete")} disabled={actionBusy} onClick={onDelete}><Trash2 /></button>}</div></article>;
}

function EmptyHistory({ language }: { language: AppLanguage }) {
  const t = translator(language);
  return <div className="empty-state"><History /><strong>{t("emptyHistory")}</strong></div>;
}

function SettingsPage({ data, language, isBusy, onSave, onRefresh, onToast, onOpenOnboarding }: {
  data: BootstrapState;
  language: AppLanguage;
  isBusy: boolean;
  onSave: (settings: AppSettings) => Promise<void>;
  onRefresh: () => Promise<BootstrapState>;
  onToast: ToastHandler;
  onOpenOnboarding: () => void;
}) {
  const t = translator(language);
  const [draft, setDraft] = useState(data.settings);
  const [apiKey, setApiKey] = useState("");
  const [keyBusy, setKeyBusy] = useState(false);
  const [shortcutBusy, setShortcutBusy] = useState(false);
  const [diagnosticBusy, setDiagnosticBusy] = useState(false);
  const [microphoneBusy, setMicrophoneBusy] = useState(false);
  const [accessibilityBusy, setAccessibilityBusy] = useState(false);
  const [saveBusy, setSaveBusy] = useState(false);
  const controlsDisabled = isBusy || keyBusy || shortcutBusy || diagnosticBusy || microphoneBusy || accessibilityBusy || saveBusy;
  const shortcutButtonDisabled = isBusy || keyBusy || diagnosticBusy || microphoneBusy || accessibilityBusy || saveBusy;
  useShortcutCapture(shortcutBusy, setShortcutBusy, (binding) => setDraft((current) => ({ ...current, dictationShortcut: binding })), (message) => onToast(errorMessage(message, language), "error"));

  useEffect(() => setDraft(data.settings), [data.settings]);
  useEffect(() => {
    let disposed = false;
    void isEnabled()
      .then((enabled) => {
        if (disposed) return;
        setDraft((current) => current.launchAtLogin === data.settings.launchAtLogin ? { ...current, launchAtLogin: enabled } : current);
      })
      .catch(() => undefined);
    return () => { disposed = true; };
  }, [data.settings.launchAtLogin]);

  const chooseFolder = async () => {
    try {
      const selected = await open({ directory: true, multiple: false, defaultPath: draft.outputDirectory ?? data.defaultOutputDirectory });
      if (selected) setDraft({ ...draft, outputDirectory: selected });
    } catch (reason) {
      onToast(errorMessage(reason, language), "error");
    }
  };

  return <div className="page settings-page"><header className="page-header"><div><span className="eyebrow">AIDOO WHISPER LITE</span><h1>{t("settings")}</h1><p>{t("version")} {data.appVersion}</p></div><button className="secondary-button" disabled={controlsDisabled} title={isBusy ? t("finishDictationFirst") : undefined} onClick={onOpenOnboarding}><Sparkles />{t("openOnboarding")}</button></header>
    <SettingsSection icon={<KeyRound />} title={t("apiTitle")}>
      <p className="section-help">{t("apiHelp")}</p><p className="instruction-note"><CircleHelp />{t("apiSteps")}</p>
      <div className="api-row"><input type="password" aria-label={t("apiTitle")} autoComplete="new-password" spellCheck={false} value={apiKey} disabled={controlsDisabled} onChange={(event) => setApiKey(event.target.value)} placeholder={data.hasApiKey ? "••••••••••••••••••" : "sk-…"} /><button className="secondary-button" disabled={controlsDisabled} onClick={async () => { try { await openUrl("https://platform.openai.com/api-keys"); } catch (reason) { onToast(errorMessage(reason, language), "error"); } }}><ExternalLink />{t("createKey")}</button><button className="primary-button" disabled={controlsDisabled || !apiKey.trim()} onClick={async () => { setKeyBusy(true); try { await invoke("save_api_key", { apiKey }); setApiKey(""); await onRefresh(); onToast(t("keySaved")); } catch (reason) { onToast(errorMessage(reason, language), "error"); } finally { setKeyBusy(false); } }}>{keyBusy ? <LoaderCircle className="spin" /> : <ShieldCheck />}{t("verifySave")}</button></div>
      {data.hasApiKey && <button className="text-button danger" disabled={controlsDisabled} onClick={async () => { setKeyBusy(true); try { await invoke("delete_api_key"); await onRefresh(); onToast(t("deleted")); } catch (reason) { onToast(errorMessage(reason, language), "error"); } finally { setKeyBusy(false); } }}>{t("removeKey")}</button>}
    </SettingsSection>
    <SettingsSection icon={<Languages />} title={t("modelLanguage")}>
      <ModelPicker settings={draft} language={language} disabled={controlsDisabled} onChange={setDraft} />
      <SettingRow title={t("interfaceLanguage")}><select aria-label={t("interfaceLanguage")} value={draft.uiLanguage} disabled={controlsDisabled} onChange={(event) => setDraft({ ...draft, uiLanguage: event.target.value as AppSettings["uiLanguage"] })}><option value="auto">{t("automatic")}</option><option value="bg">Български</option><option value="en">English</option></select></SettingRow>
    </SettingsSection>
    <SettingsSection icon={<Mic />} title={t("microphone")}>
      <SettingRow title={t("microphone")}><select aria-label={t("microphone")} value={draft.microphoneName ?? ""} disabled={controlsDisabled} onChange={(event) => setDraft({ ...draft, microphoneName: event.target.value || null })}><option value="">{t("systemDefault")}</option>{data.microphones.map((item) => <option key={item} value={item}>{item}</option>)}</select></SettingRow>
      <SettingRow title={t("automaticMicrophoneFallback")} detail={t("automaticMicrophoneFallbackHelp")}><Toggle label={t("automaticMicrophoneFallback")} checked={draft.automaticMicrophoneFallback} disabled={controlsDisabled} onChange={(automaticMicrophoneFallback) => setDraft({ ...draft, automaticMicrophoneFallback })} /></SettingRow>
      <SettingRow title={t("testMicrophone")} detail={t("microphoneTestHelp")}><button className="secondary-button" disabled={controlsDisabled || !data.microphones.length} onClick={async () => { setMicrophoneBusy(true); try { const probe = await invoke<MicrophoneProbe>("test_microphone", { microphoneName: draft.microphoneName, automaticFallback: draft.automaticMicrophoneFallback }); if (!probe.heardAudio) throw new Error(t("microphoneSilent")); onToast(probe.usedFallback ? t("microphoneFallback", { name: probe.deviceName }) : t("microphoneOk"), probe.usedFallback ? "warning" : "success"); } catch (reason) { onToast(errorMessage(reason, language), "error"); } finally { setMicrophoneBusy(false); } }}>{microphoneBusy ? <LoaderCircle className="spin" /> : <AudioLines />}{t("testMicrophone")}</button></SettingRow>
      <SettingRow title={t("accessibility")} detail={data.accessibilityGranted ? t("ready") : t("accessibilityHelp")}><div className="inline-actions"><button className="secondary-button" disabled={controlsDisabled} onClick={async () => { try { await invoke("open_accessibility_settings"); } catch (reason) { onToast(errorMessage(reason, language), "error"); } }}>{t("grant")}</button><button className="secondary-button" disabled={controlsDisabled} onClick={async () => { setAccessibilityBusy(true); try { await invoke("refresh_accessibility_status"); await onRefresh(); } catch (reason) { onToast(errorMessage(reason, language), "error"); } finally { setAccessibilityBusy(false); } }}>{accessibilityBusy ? <LoaderCircle className="spin" /> : data.accessibilityGranted ? <Check /> : <RefreshCw />}{t("refresh")}</button></div></SettingRow>
      <SettingRow title={t("shortcut")} detail={formatShortcut(draft.dictationShortcut)}><button className="secondary-button" disabled={shortcutButtonDisabled} onClick={async () => { if (shortcutBusy) { try { await invoke("cancel_shortcut_capture"); } catch (reason) { onToast(errorMessage(reason, language), "error"); } finally { setShortcutBusy(false); } return; } setShortcutBusy(true); try { await invoke("begin_shortcut_capture"); } catch (reason) { setShortcutBusy(false); onToast(errorMessage(reason, language), "error"); } }}>{shortcutBusy ? <X /> : null}{shortcutBusy ? t("cancel") : t("changeShortcut")}</button></SettingRow>
      <SettingRow title={t("autoPaste")} detail={t("autoPasteHelp")}><Toggle label={t("autoPaste")} checked={draft.autoPaste} disabled={controlsDisabled} onChange={(autoPaste) => setDraft({ ...draft, autoPaste })} /></SettingRow>
    </SettingsSection>
    <SettingsSection icon={<FolderOpen />} title={t("storage")}>
      <StorageControls settings={draft} language={language} outputPath={draft.outputDirectory ?? data.defaultOutputDirectory} disabled={controlsDisabled} onChange={setDraft} onChooseFolder={chooseFolder} />
    </SettingsSection>
    <SettingsSection icon={<Power />} title={t("startup")}>
      <SettingRow title={t("launchAtLogin")}><Toggle label={t("launchAtLogin")} checked={draft.launchAtLogin} disabled={controlsDisabled} onChange={(launchAtLogin) => setDraft({ ...draft, launchAtLogin })} /></SettingRow>
    </SettingsSection>
    <SettingsSection icon={<ShieldCheck />} title={t("diagnostics")}>
      <p className="section-help">{t("diagnosticsHelp")}</p><div className="inline-actions"><button className="secondary-button" disabled={controlsDisabled} onClick={async () => { setDiagnosticBusy(true); try { const path = await invoke<string>("create_diagnostic_bundle"); await invoke("open_local_path", { path, reveal: true }); } catch (reason) { onToast(errorMessage(reason, language), "error"); } finally { setDiagnosticBusy(false); } }}>{diagnosticBusy ? <LoaderCircle className="spin" /> : <FileText />}{t("createDiagnostics")}</button><button className="secondary-button" disabled={controlsDisabled} onClick={async () => { try { await openUrl("https://github.com/bkafelov-aidoo/AIDOO-Whisper-Lite/issues"); } catch (reason) { onToast(errorMessage(reason, language), "error"); } }}><ExternalLink />{t("openSupport")}</button></div>
    </SettingsSection>
    <footer className="settings-footer"><button className="primary-button large" disabled={controlsDisabled} title={isBusy ? t("finishDictationFirst") : undefined} onClick={async () => { setSaveBusy(true); try { await onSave(draft); } finally { setSaveBusy(false); } }}>{saveBusy ? <LoaderCircle className="spin" /> : <Check />}{t("save")}</button></footer>
  </div>;
}

function SettingsSection({ icon, title, children }: { icon: React.ReactNode; title: string; children: React.ReactNode }) {
  return <section className="settings-section"><header><div>{icon}</div><h2>{title}</h2></header><div className="settings-body">{children}</div></section>;
}

function SettingRow({ title, detail, children }: { title: string; detail?: string; children: React.ReactNode }) {
  return <div className="setting-row"><div><strong>{title}</strong>{detail && <span>{detail}</span>}</div>{children}</div>;
}

function Toggle({ label, checked, disabled = false, onChange }: { label: string; checked: boolean; disabled?: boolean; onChange: (value: boolean) => void }) {
  return <button type="button" role="switch" aria-label={label} aria-checked={checked} disabled={disabled} className={`toggle ${checked ? "on" : ""}`} onClick={() => onChange(!checked)}><span /></button>;
}

function ModelPicker({ settings, language, disabled = false, onChange }: { settings: AppSettings; language: AppLanguage; disabled?: boolean; onChange: (settings: AppSettings) => void }) {
  const t = translator(language);
  return <div className="model-language-grid"><button type="button" aria-pressed={settings.model === "gpt-4o-mini-transcribe"} disabled={disabled} className={`model-option ${settings.model === "gpt-4o-mini-transcribe" ? "selected" : ""}`} onClick={() => onChange({ ...settings, model: "gpt-4o-mini-transcribe" })}><span><Sparkles /></span><strong>{t("economy")}</strong><small>{t("economyHelp")}</small><em>$0.003 / {t("perMinute")}</em></button><button type="button" aria-pressed={settings.model === "gpt-transcribe"} disabled={disabled} className={`model-option ${settings.model === "gpt-transcribe" ? "selected" : ""}`} onClick={() => onChange({ ...settings, model: "gpt-transcribe" })}><span><ShieldCheck /></span><strong>{t("accuracy")}</strong><small>{t("accuracyHelp")}</small><em>$0.0045 / {t("perMinute")}</em></button><label className="language-select"><span>{t("modelLanguage")}</span><select value={settings.language} disabled={disabled} onChange={(event) => onChange({ ...settings, language: event.target.value })}><option value="auto">{t("autoLanguage")}</option><option value="bg">{t("bulgarian")}</option><option value="en">{t("english")}</option><option value="de">Deutsch</option><option value="es">Español</option><option value="fr">Français</option><option value="it">Italiano</option></select></label></div>;
}

function StorageControls({ settings, language, outputPath, disabled = false, onChange, onChooseFolder }: { settings: AppSettings; language: AppLanguage; outputPath: string; disabled?: boolean; onChange: (settings: AppSettings) => void; onChooseFolder: () => void }) {
  const t = translator(language);
  return <div className="storage-controls"><SettingRow title={t("saveFlac")} detail={t("saveFlacHelp")}><Toggle label={t("saveFlac")} checked={settings.saveAudio} disabled={disabled} onChange={(saveAudio) => onChange({ ...settings, saveAudio })} /></SettingRow><SettingRow title={t("saveTxt")} detail={t("saveTxtHelp")}><Toggle label={t("saveTxt")} checked={settings.saveText} disabled={disabled} onChange={(saveText) => onChange({ ...settings, saveText })} /></SettingRow><SettingRow title={t("saveHistory")} detail={t("saveHistoryHelp")}><Toggle label={t("saveHistory")} checked={settings.historyEnabled} disabled={disabled} onChange={(historyEnabled) => onChange({ ...settings, historyEnabled })} /></SettingRow><div className="folder-picker"><div><strong>{t("folder")}</strong><span title={outputPath}>{outputPath}</span></div><button className="secondary-button" disabled={disabled} onClick={onChooseFolder}><FolderOpen />{t("chooseFolder")}</button></div></div>;
}

function Onboarding({ data, language, isBusy, onData, onPersist, onRefresh, onClose, onToast }: { data: BootstrapState; language: AppLanguage; isBusy: boolean; onData: React.Dispatch<React.SetStateAction<BootstrapState | null>>; onPersist: (settings: AppSettings) => Promise<AppSettings>; onRefresh: () => Promise<BootstrapState>; onClose: () => void; onToast: ToastHandler }) {
  const t = translator(language);
  const [step, setStep] = useState(0);
  const [draft, setDraft] = useState(data.settings);
  const [apiKey, setApiKey] = useState("");
  const [busy, setBusy] = useState(false);
  const [micTested, setMicTested] = useState(data.settings.onboardingComplete && data.microphones.length > 0);
  const [shortcutBusy, setShortcutBusy] = useState(false);
  const controlsDisabled = busy || isBusy || shortcutBusy;
  const dialogRef = useDialogFocus(onClose, !shortcutBusy);
  useShortcutCapture(shortcutBusy, setShortcutBusy, (binding) => setDraft((current) => ({ ...current, dictationShortcut: binding })), (message) => onToast(errorMessage(message, language), "error"));
  const steps = [t("stepAccount"), t("stepPermissions"), t("stepShortcut"), t("stepPreferences"), t("stepStorage"), t("stepReady")];

  const saveDraft = async () => { const saved = await onPersist(draft); setDraft(saved); return saved; };
  const next = async () => { setBusy(true); try { await saveDraft(); setStep((value) => Math.min(steps.length - 1, value + 1)); } catch (reason) { onToast(errorMessage(reason, language), "error"); } finally { setBusy(false); } };
  const chooseFolder = async () => {
    try {
      const selected = await open({ directory: true, multiple: false, defaultPath: draft.outputDirectory ?? data.defaultOutputDirectory });
      if (selected) setDraft({ ...draft, outputDirectory: selected });
    } catch (reason) {
      onToast(errorMessage(reason, language), "error");
    }
  };
  const missing = !data.hasApiKey ? t("keyRequired") : !data.microphones.length || !micTested ? t("micRequired") : !data.accessibilityGranted ? t("accessRequired") : null;

  return <div className="modal-backdrop"><section ref={dialogRef} className="onboarding-modal" role="dialog" aria-modal="true" aria-label={steps[step]} tabIndex={-1}><header className="onboarding-header"><div className="brand compact"><img src="/app-icon.png" alt="" /><div><strong>AIDOO</strong><span>Whisper Lite</span></div></div><button className="close-button" aria-label={t("closeContinueLater")} onClick={onClose}><X /></button></header><div className="stepper" role="list">{steps.map((name, index) => <div key={name} role="listitem" aria-current={index === step ? "step" : undefined} className={`${index === step ? "active" : ""} ${index < step ? "done" : ""}`}><i>{index < step ? <Check /> : index + 1}</i><span>{name}</span></div>)}</div><div className="onboarding-content">
    {step === 0 && <div className="onboarding-step"><span className="step-icon"><KeyRound /></span><h1>{t("welcomeTitle")}</h1><p>{t("welcomeBody")}</p><div className="instruction-card"><CircleHelp /><span>{t("apiSteps")}</span></div><button className="secondary-button wide" disabled={controlsDisabled} onClick={async () => { try { await openUrl("https://platform.openai.com/api-keys"); } catch (reason) { onToast(errorMessage(reason, language), "error"); } }}><ExternalLink />{t("createKey")}</button><div className="api-input"><input type="password" aria-label={t("apiTitle")} autoComplete="new-password" spellCheck={false} placeholder="sk-…" value={apiKey} disabled={controlsDisabled} onChange={(event) => setApiKey(event.target.value)} /><button className="primary-button" disabled={controlsDisabled || !apiKey.trim()} onClick={async () => { setBusy(true); try { await invoke("save_api_key", { apiKey }); setApiKey(""); const nextData = await onRefresh(); onData(nextData); onToast(t("keySaved")); } catch (reason) { onToast(errorMessage(reason, language), "error"); } finally { setBusy(false); } }}>{busy ? <LoaderCircle className="spin" /> : <ShieldCheck />}{t("verifySave")}</button></div>{data.hasApiKey && <div className="success-note"><Check />{t("keySaved")}</div>}</div>}
    {step === 1 && <div className="onboarding-step"><span className="step-icon"><Mic /></span><h1>{t("stepPermissions")}</h1><p>{t("accessibilityHelp")}</p><div className="permission-list"><article className={micTested ? "ready" : ""}><div><Mic /></div><span><strong>{t("microphone")}</strong><small>{draft.microphoneName ?? t("systemDefault")} · {t("microphoneTestHelp")}</small></span><select aria-label={t("microphone")} value={draft.microphoneName ?? ""} disabled={controlsDisabled} onChange={(event) => { setMicTested(false); setDraft({ ...draft, microphoneName: event.target.value || null }); }}><option value="">{t("systemDefault")}</option>{data.microphones.map((item) => <option key={item} value={item}>{item}</option>)}</select><button className="secondary-button" disabled={controlsDisabled || !data.microphones.length} onClick={async () => { setBusy(true); try { const probe = await invoke<MicrophoneProbe>("test_microphone", { microphoneName: draft.microphoneName, automaticFallback: draft.automaticMicrophoneFallback }); if (!probe.heardAudio) throw new Error(t("microphoneSilent")); setMicTested(true); onToast(probe.usedFallback ? t("microphoneFallback", { name: probe.deviceName }) : t("microphoneOk"), probe.usedFallback ? "warning" : "success"); } catch (reason) { setMicTested(false); onToast(errorMessage(reason, language), "error"); } finally { setBusy(false); } }}>{busy ? <LoaderCircle className="spin" /> : <AudioLines />}{t("testMicrophone")}</button></article><article className={data.accessibilityGranted ? "ready" : ""}><div><ShieldCheck /></div><span><strong>{t("accessibility")}</strong><small>{t("accessibilityHelp")}</small></span><button className="secondary-button" disabled={controlsDisabled} onClick={async () => { try { await invoke("open_accessibility_settings"); } catch (reason) { onToast(errorMessage(reason, language), "error"); } }}>{t("grant")}</button><button className="secondary-button" disabled={controlsDisabled} onClick={async () => { setBusy(true); try { await invoke("refresh_accessibility_status"); const nextData = await onRefresh(); onData(nextData); } catch (reason) { onToast(errorMessage(reason, language), "error"); } finally { setBusy(false); } }}>{data.accessibilityGranted ? <Check /> : <RefreshCw />}{t("refresh")}</button></article></div><SettingRow title={t("automaticMicrophoneFallback")} detail={t("automaticMicrophoneFallbackHelp")}><Toggle label={t("automaticMicrophoneFallback")} checked={draft.automaticMicrophoneFallback} disabled={controlsDisabled} onChange={(automaticMicrophoneFallback) => { setMicTested(false); setDraft({ ...draft, automaticMicrophoneFallback }); }} /></SettingRow></div>}
    {step === 2 && <div className="onboarding-step"><span className="step-icon"><AudioLines /></span><h1>{t("shortcut")}</h1><p>{t("holdShortcut", { shortcut: formatShortcut(draft.dictationShortcut) })}</p><button className={`shortcut-capture ${shortcutBusy ? "listening" : ""}`} disabled={controlsDisabled} onClick={async () => { setShortcutBusy(true); try { await invoke("begin_shortcut_capture"); } catch (reason) { setShortcutBusy(false); onToast(errorMessage(reason, language), "error"); } }}>{shortcutBusy ? <><AudioLines />{t("pressShortcut")}</> : <><kbd>{formatShortcut(draft.dictationShortcut)}</kbd>{t("changeShortcut")}</>}</button>{shortcutBusy && <button className="text-button" onClick={async () => { try { await invoke("cancel_shortcut_capture"); } catch (reason) { onToast(errorMessage(reason, language), "error"); } finally { setShortcutBusy(false); } }}>{t("cancel")}</button>}</div>}
    {step === 3 && <div className="onboarding-step"><span className="step-icon"><Languages /></span><h1>{t("modelLanguage")}</h1><p>{language === "bg" ? "Изберете баланс между цена и точност и задайте език, за да избегнете автоматичното разпознаване." : "Choose your cost/accuracy balance and set a language to skip automatic detection."}</p><ModelPicker settings={draft} language={language} disabled={controlsDisabled} onChange={setDraft} /></div>}
    {step === 4 && <div className="onboarding-step"><span className="step-icon"><FolderOpen /></span><h1>{t("storage")}</h1><p>{language === "bg" ? "Всеки тип съхранение се управлява отделно. Потребителските файлове никога не се изтриват автоматично." : "Each storage type is controlled separately. User files are never deleted automatically."}</p><StorageControls settings={draft} language={language} outputPath={draft.outputDirectory ?? data.defaultOutputDirectory} disabled={controlsDisabled} onChange={setDraft} onChooseFolder={chooseFolder} /></div>}
    {step === 5 && <div className="onboarding-step ready-step"><span className={`step-icon ${missing ? "warning" : "success"}`}>{missing ? <AlertCircle /> : <Check />}</span><h1>{missing ?? t("allReady")}</h1><p>{missing ? t("onboardingIncomplete") : t("readyBody")}</p><div className="ready-summary"><span><KeyRound /><strong>OpenAI</strong><em>{data.hasApiKey ? "✓" : "—"}</em></span><span><Mic /><strong>{t("microphone")}</strong><em>{micTested ? "✓" : "—"}</em></span><span><ShieldCheck /><strong>Accessibility</strong><em>{data.accessibilityGranted ? "✓" : "—"}</em></span><span><AudioLines /><strong>{t("shortcut")}</strong><em>{formatShortcut(draft.dictationShortcut)}</em></span></div></div>}
  </div><footer className="onboarding-footer"><button className="text-button" onClick={onClose}>{t("closeContinueLater")}</button><div>{step > 0 && <button className="secondary-button" disabled={controlsDisabled} onClick={() => setStep((value) => value - 1)}><ChevronLeft />{t("back")}</button>}{step < steps.length - 1 ? <button className="primary-button" disabled={controlsDisabled} onClick={next}>{t("continue")}<ChevronRight /></button> : <button className="primary-button" disabled={controlsDisabled || Boolean(missing)} onClick={async () => { setBusy(true); try { const completed = { ...draft, onboardingComplete: true }; await onPersist(completed); await onRefresh(); onClose(); } catch (reason) { onToast(errorMessage(reason, language), "error"); } finally { setBusy(false); } }}><Check />{t("finish")}</button>}</div></footer></section></div>;
}

function useShortcutCapture(active: boolean, setActive: (active: boolean) => void, onCaptured: (binding: ShortcutBinding) => void, onError: (message: string) => void) {
  const activeRef = useRef(active);
  const capturedRef = useRef(onCaptured);
  const errorRef = useRef(onError);
  activeRef.current = active;
  capturedRef.current = onCaptured;
  errorRef.current = onError;

  useEffect(() => {
    let disposed = false;
    const unlisten: Array<() => void> = [];
    const register = (promise: Promise<() => void>) => {
      void promise.then((fn) => disposed ? fn() : unlisten.push(fn));
    };
    register(listen<{ binding: ShortcutBinding }>("shortcut:captured", ({ payload }) => { capturedRef.current(payload.binding); setActive(false); }));
    register(listen<string>("shortcut:capture-error", ({ payload }) => errorRef.current(payload)));
    register(listen("shortcut:capture-cancelled", () => setActive(false)));
    return () => {
      disposed = true;
      unlisten.forEach((fn) => fn());
      if (activeRef.current) void invoke("cancel_shortcut_capture");
    };
  }, [setActive]);
}

function DeleteDialog({ entry, language, onCancel, onDelete }: { entry: TranscriptEntry; language: AppLanguage; onCancel: () => void; onDelete: (deleteFiles: boolean) => void }) {
  const t = translator(language);
  const dialogRef = useDialogFocus(onCancel);
  return <div className="modal-backdrop small"><section ref={dialogRef} className="confirm-dialog" role="alertdialog" aria-modal="true" aria-labelledby="delete-dialog-title" tabIndex={-1}><button className="close-button" aria-label={t("cancel")} onClick={onCancel}><X /></button><span className="danger-icon"><Trash2 /></span><h2 id="delete-dialog-title">{t("deleteQuestion")}</h2><p>{entry.text}</p><button className="secondary-button" onClick={() => onDelete(false)}>{t("historyOnly")}</button><button className="danger-button" onClick={() => onDelete(true)}>{t("historyAndFiles")}</button></section></div>;
}

function useDialogFocus(onClose: () => void, closeOnEscape = true) {
  const dialogRef = useRef<HTMLElement>(null);
  const closeRef = useRef(onClose);
  const escapeRef = useRef(closeOnEscape);
  closeRef.current = onClose;
  escapeRef.current = closeOnEscape;

  useEffect(() => {
    const dialog = dialogRef.current;
    const previousFocus = document.activeElement instanceof HTMLElement ? document.activeElement : null;
    if (!dialog) return;
    dialog.focus();

    const onKeyDown = (event: KeyboardEvent) => {
      if (event.key === "Escape" && escapeRef.current) {
        event.preventDefault();
        closeRef.current();
        return;
      }
      if (event.key !== "Tab") return;
      const focusable = Array.from(dialog.querySelectorAll<HTMLElement>('button:not([disabled]), input:not([disabled]), select:not([disabled]), [href], [tabindex]:not([tabindex="-1"])'));
      if (!focusable.length) {
        event.preventDefault();
        dialog.focus();
        return;
      }
      const first = focusable[0];
      const last = focusable[focusable.length - 1];
      if (event.shiftKey && (document.activeElement === first || document.activeElement === dialog)) {
        event.preventDefault();
        last.focus();
      } else if (!event.shiftKey && document.activeElement === last) {
        event.preventDefault();
        first.focus();
      }
    };

    dialog.addEventListener("keydown", onKeyDown);
    return () => {
      dialog.removeEventListener("keydown", onKeyDown);
      previousFocus?.focus();
    };
  }, []);

  return dialogRef;
}

function StatusPill({ ready, label }: { ready: boolean; label: string }) {
  return <span className={`status-pill ${ready ? "ready" : "attention"}`}><i />{label}</span>;
}

function appIsReady(data: BootstrapState) {
  return data.settings.onboardingComplete
    && data.hasApiKey
    && data.microphones.length > 0
    && data.accessibilityGranted
    && !data.failedRecording;
}

function settingsMatch(left: AppSettings, right: AppSettings) {
  return JSON.stringify(left) === JSON.stringify(right);
}

function formatShortcut(binding: ShortcutBinding) {
  if (binding.code === "alt_gr") return "⌥ Right";
  const names: Record<string, string> = { meta: "⌘", shift: "⇧", alt: "⌥", control: "⌃", fn: "fn" };
  const code = binding.code.replace("key_", "").replace("num_", "").replace(/_/g, " ").toUpperCase();
  return [...binding.modifiers.map((item) => names[item] ?? item), code].join(" ");
}

function formatDuration(seconds: number) {
  const total = Math.max(0, Math.round(seconds));
  return `${Math.floor(total / 60)}:${String(total % 60).padStart(2, "0")}`;
}

function formatDate(value: string, language: AppLanguage) {
  const date = new Date(value);
  if (Number.isNaN(date.getTime())) return value;
  return new Intl.DateTimeFormat(language === "bg" ? "bg-BG" : "en-US", { dateStyle: "medium", timeStyle: "short" }).format(date);
}

function modelLabel(model: string, language: AppLanguage) {
  if (model === "gpt-4o-mini-transcribe") return language === "bg" ? "Икономичен" : "Economy";
  if (model === "gpt-transcribe") return language === "bg" ? "Максимална точност" : "Maximum accuracy";
  return model;
}
