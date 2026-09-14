import { useCallback, useEffect, useRef, useState } from "react";
import { invoke } from "@tauri-apps/api/core";
import { listen } from "@tauri-apps/api/event";
import { open } from "@tauri-apps/plugin-dialog";
import { openPath, openUrl } from "@tauri-apps/plugin-opener";
import { disable, enable } from "@tauri-apps/plugin-autostart";
import { check, type Update } from "@tauri-apps/plugin-updater";
import { relaunch } from "@tauri-apps/plugin-process";
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
  RefreshCw,
  RotateCcw,
  Settings,
  ShieldCheck,
  Sparkles,
  Trash2,
  X,
} from "lucide-react";
import { errorMessage, progressLabel, resolveLanguage, translator } from "./i18n";
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
  const [availableUpdate, setAvailableUpdate] = useState<Update | null>(null);
  const [updateMessage, setUpdateMessage] = useState<string | null>(null);
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
      setData(next);
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
    const previous = Number(localStorage.getItem("aidoo-lite-update-check") ?? 0);
    if (Date.now() - previous < 86_400_000) return;
    localStorage.setItem("aidoo-lite-update-check", String(Date.now()));
    void check().then((update) => update && setAvailableUpdate(update)).catch(() => undefined);
  }, []);

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

  const checkUpdates = async () => {
    setUpdateMessage(null);
    try {
      const update = await check();
      setAvailableUpdate(update);
      setUpdateMessage(update ? t("updateAvailable", { version: update.version }) : t("upToDate"));
    } catch (reason) {
      setUpdateMessage(errorMessage(reason, language));
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
    <div className="app-shell">
      <aside className="sidebar">
        <div className="brand">
          <img src="/app-icon.png" alt="" />
          <div><strong>AIDOO</strong><span>Whisper Lite</span></div>
        </div>
        <nav>
          <NavButton active={page === "dictation"} icon={<Mic />} label={t("dictation")} onClick={() => setPage("dictation")} />
          <NavButton active={page === "history"} icon={<History />} label={t("history")} badge={data.history.length || undefined} onClick={() => setPage("history")} />
          <NavButton active={page === "settings"} icon={<Settings />} label={t("settings")} onClick={() => setPage("settings")} />
        </nav>
        <div className={`sidebar-status ${data.settings.onboardingComplete ? "ready" : "attention"}`}>
          <i />
          <span>{data.settings.onboardingComplete ? t("ready") : t("notReady")}</span>
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
              try { await openPath(path); }
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
              try { await openPath(path); }
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
            availableUpdate={availableUpdate}
            updateMessage={updateMessage}
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
            onCheckUpdates={checkUpdates}
            onInstallUpdate={async () => {
              if (!availableUpdate) return;
              if (isBusy) {
                showToast(t("updateBusy"), "warning");
                return;
              }
              let updateLock = false;
              try {
                await invoke("begin_update_install");
                updateLock = true;
                await availableUpdate.downloadAndInstall();
                await relaunch();
              } catch (reason) {
                showToast(errorMessage(reason, language), "error");
              } finally {
                if (updateLock) await invoke("cancel_update_install").catch(() => undefined);
              }
            }}
            onToast={showToast}
            onOpenOnboarding={() => setShowOnboarding(true)}
          />
        )}
      </main>

      {showOnboarding && (
        <Onboarding
          data={data}
          language={language}
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
              showToast(errorMessage(reason, language), "error");
            }
          }}
        />
      )}

      {toast && <div className={`toast ${toast.tone}`}>{toast.tone === "success" ? <Check /> : <AlertCircle />}{toast.message}</div>}
    </div>
  );
}

function NavButton({ active, icon, label, badge, onClick }: { active: boolean; icon: React.ReactNode; label: string; badge?: number; onClick: () => void }) {
  return <button className={active ? "active" : ""} onClick={onClick}>{icon}<span>{label}</span>{badge ? <em>{badge}</em> : null}</button>;
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
  return (
    <div className="page dashboard">
      <header className="page-header"><div><span className="eyebrow">AIDOO WHISPER LITE</span><h1>{t("dictation")}</h1><p>{t("tagline")}</p></div><StatusPill ready={data.settings.onboardingComplete} label={data.settings.onboardingComplete ? t("ready") : t("notReady")} /></header>
      {!data.settings.onboardingComplete && (
        <section className="setup-banner"><AlertCircle /><div><strong>{t("notReady")}</strong><span>{t("onboardingIncomplete")}</span></div><button onClick={onOpenOnboarding}>{t("openOnboarding")}<ChevronRight /></button></section>
      )}
      <section className={`dictation-hero ${data.recording.state}`}>
        <div className="hero-glow" />
        <div className="shortcut-key"><span>{shortcut}</span><small>HOLD</small></div>
        <div className="hero-copy"><span className="eyebrow">PUSH TO TALK</span><h2>{data.recording.state === "recording" ? t("recording") : data.recording.state === "transcribing" ? t("transcribing") : t("holdShortcut", { shortcut })}</h2><p>{data.recording.state === "transcribing" ? progressLabel(data.recording.progress.stage, language) : t("autoPasteHelp")}</p></div>
        <button className="hero-action" disabled={!data.settings.onboardingComplete || (isBusy && data.recording.state !== "recording")} onClick={onTest}>{data.recording.state === "recording" ? <><AudioLines />{t("stopTest")}</> : <><Play />{t("startTest")}</>}</button>
      </section>
      {data.failedRecording && <FailedCard failed={data.failedRecording} language={language} disabled={isBusy} onRetry={onRetry} onDelete={onDeleteFailed} />}
      <section className="section-card">
        <header><div><h3>{t("recent")}</h3><p>{t("saveHistoryHelp")}</p></div><button className="text-button" onClick={() => data.history[0] && onCopy(data.history[0].text)} disabled={!data.history[0]}><Clipboard />{t("copy")}</button></header>
        {data.history.length ? data.history.slice(0, 4).map((entry) => <HistoryRow key={entry.id} entry={entry} language={language} actionBusy={isBusy} onCopy={onCopy} onOpen={onOpen} onRetranscribe={onRetranscribe} />) : <EmptyHistory language={language} />}
      </section>
    </div>
  );
}

function FailedCard({ failed, language, disabled, onRetry, onDelete }: { failed: FailedRecording; language: AppLanguage; disabled: boolean; onRetry: () => void; onDelete: () => void }) {
  const t = translator(language);
  return <section className="failed-card"><AlertCircle /><div><strong>{t("failedTitle")}</strong><span>{t("failedBody")}</span><small>{errorMessage(failed.error, language)}</small></div><button className="secondary-button" disabled={disabled} onClick={onRetry}><RotateCcw />{t("retry")}</button><button className="icon-button danger" disabled={disabled} onClick={onDelete} aria-label={t("delete")}><Trash2 /></button></section>;
}

function HistoryPage({ history, language, isBusy, onCopy, onOpen, onRetranscribe, onDelete }: { history: TranscriptEntry[]; language: AppLanguage; isBusy: boolean; onCopy: (text: string) => void; onOpen: (path: string) => void; onRetranscribe: (id: string) => void; onDelete: (entry: TranscriptEntry) => void }) {
  const t = translator(language);
  return <div className="page"><header className="page-header"><div><span className="eyebrow">LOCAL</span><h1>{t("history")}</h1><p>{t("saveHistoryHelp")}</p></div><span className="count-pill">{history.length} / 10</span></header><section className="section-card history-card">{history.length ? history.map((entry) => <HistoryRow key={entry.id} entry={entry} language={language} expanded actionBusy={isBusy} onCopy={onCopy} onOpen={onOpen} onRetranscribe={onRetranscribe} onDelete={() => onDelete(entry)} />) : <EmptyHistory language={language} />}</section></div>;
}

function HistoryRow({ entry, language, expanded = false, actionBusy = false, onCopy, onOpen, onRetranscribe, onDelete }: { entry: TranscriptEntry; language: AppLanguage; expanded?: boolean; actionBusy?: boolean; onCopy: (text: string) => void; onOpen: (path: string) => void; onRetranscribe: (id: string) => void; onDelete?: () => void }) {
  const t = translator(language);
  return <article className={`history-row ${expanded ? "expanded" : ""}`}><div className="history-icon"><AudioLines /></div><div className="history-copy"><p>{entry.text}</p><span><Clock3 />{formatDate(entry.createdAt, language)} · {formatDuration(entry.durationSeconds)} · {modelLabel(entry.model, language)}</span></div><div className="history-actions"><button title={t("copy")} aria-label={t("copy")} onClick={() => onCopy(entry.text)}><Clipboard /></button>{entry.audioPath && <button title={t("retranscribe")} aria-label={t("retranscribe")} disabled={actionBusy} onClick={() => onRetranscribe(entry.id)}><RotateCcw /></button>}{entry.audioPath && <button title={t("openAudio")} aria-label={t("openAudio")} onClick={() => onOpen(entry.audioPath!)}><FileAudio /></button>}{entry.textPath && <button title={t("openText")} aria-label={t("openText")} onClick={() => onOpen(entry.textPath!)}><FileText /></button>}{onDelete && <button className="danger" title={t("delete")} aria-label={t("delete")} disabled={actionBusy} onClick={onDelete}><Trash2 /></button>}</div></article>;
}

function EmptyHistory({ language }: { language: AppLanguage }) {
  const t = translator(language);
  return <div className="empty-state"><History /><strong>{t("emptyHistory")}</strong></div>;
}

function SettingsPage({ data, language, availableUpdate, updateMessage, isBusy, onSave, onRefresh, onCheckUpdates, onInstallUpdate, onToast, onOpenOnboarding }: {
  data: BootstrapState;
  language: AppLanguage;
  availableUpdate: Update | null;
  updateMessage: string | null;
  isBusy: boolean;
  onSave: (settings: AppSettings) => Promise<void>;
  onRefresh: () => Promise<BootstrapState>;
  onCheckUpdates: () => Promise<void>;
  onInstallUpdate: () => Promise<void>;
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
  useShortcutCapture(shortcutBusy, setShortcutBusy, (binding) => setDraft((current) => ({ ...current, dictationShortcut: binding })), (message) => onToast(errorMessage(message, language), "error"));

  useEffect(() => setDraft(data.settings), [data.settings]);

  const chooseFolder = async () => {
    try {
      const selected = await open({ directory: true, multiple: false, defaultPath: draft.outputDirectory ?? data.defaultOutputDirectory });
      if (selected) setDraft({ ...draft, outputDirectory: selected });
    } catch (reason) {
      onToast(errorMessage(reason, language), "error");
    }
  };

  return <div className="page settings-page"><header className="page-header"><div><span className="eyebrow">AIDOO WHISPER LITE</span><h1>{t("settings")}</h1><p>{t("version")} {data.appVersion}</p></div><button className="secondary-button" disabled={isBusy} title={isBusy ? t("finishDictationFirst") : undefined} onClick={onOpenOnboarding}><Sparkles />{t("openOnboarding")}</button></header>
    <SettingsSection icon={<KeyRound />} title={t("apiTitle")}>
      <p className="section-help">{t("apiHelp")}</p><p className="instruction-note"><CircleHelp />{t("apiSteps")}</p>
      <div className="api-row"><input type="password" value={apiKey} disabled={isBusy} onChange={(event) => setApiKey(event.target.value)} placeholder={data.hasApiKey ? "••••••••••••••••••" : "sk-…"} /><button className="secondary-button" onClick={async () => { try { await openUrl("https://platform.openai.com/api-keys"); } catch (reason) { onToast(errorMessage(reason, language), "error"); } }}><ExternalLink />{t("createKey")}</button><button className="primary-button" disabled={isBusy || keyBusy || !apiKey.trim()} onClick={async () => { setKeyBusy(true); try { await invoke("save_api_key", { apiKey }); setApiKey(""); await onRefresh(); onToast(t("keySaved")); } catch (reason) { onToast(errorMessage(reason, language), "error"); } finally { setKeyBusy(false); } }}>{keyBusy ? <LoaderCircle className="spin" /> : <ShieldCheck />}{t("verifySave")}</button></div>
      {data.hasApiKey && <button className="text-button danger" disabled={isBusy} onClick={async () => { try { await invoke("delete_api_key"); await onRefresh(); onToast(t("deleted")); } catch (reason) { onToast(errorMessage(reason, language), "error"); } }}>{t("removeKey")}</button>}
    </SettingsSection>
    <SettingsSection icon={<Languages />} title={t("modelLanguage")}>
      <ModelPicker settings={draft} language={language} onChange={setDraft} />
      <SettingRow title={t("interfaceLanguage")}><select value={draft.uiLanguage} onChange={(event) => setDraft({ ...draft, uiLanguage: event.target.value as AppSettings["uiLanguage"] })}><option value="auto">{t("automatic")}</option><option value="bg">Български</option><option value="en">English</option></select></SettingRow>
    </SettingsSection>
    <SettingsSection icon={<Mic />} title={t("microphone")}>
      <SettingRow title={t("microphone")}><select value={draft.microphoneName ?? ""} onChange={(event) => setDraft({ ...draft, microphoneName: event.target.value || null })}><option value="">{t("systemDefault")}</option>{data.microphones.map((item) => <option key={item} value={item}>{item}</option>)}</select></SettingRow>
      <SettingRow title={t("automaticMicrophoneFallback")} detail={t("automaticMicrophoneFallbackHelp")}><Toggle label={t("automaticMicrophoneFallback")} checked={draft.automaticMicrophoneFallback} onChange={(automaticMicrophoneFallback) => setDraft({ ...draft, automaticMicrophoneFallback })} /></SettingRow>
      <SettingRow title={t("testMicrophone")} detail={t("microphoneTestHelp")}><button className="secondary-button" disabled={isBusy || microphoneBusy || !data.microphones.length} onClick={async () => { setMicrophoneBusy(true); try { const probe = await invoke<MicrophoneProbe>("test_microphone", { microphoneName: draft.microphoneName, automaticFallback: draft.automaticMicrophoneFallback }); if (!probe.heardAudio) throw new Error(t("microphoneSilent")); onToast(probe.usedFallback ? t("microphoneFallback", { name: probe.deviceName }) : t("microphoneOk"), probe.usedFallback ? "warning" : "success"); } catch (reason) { onToast(errorMessage(reason, language), "error"); } finally { setMicrophoneBusy(false); } }}>{microphoneBusy ? <LoaderCircle className="spin" /> : <AudioLines />}{t("testMicrophone")}</button></SettingRow>
      <SettingRow title={t("accessibility")} detail={data.accessibilityGranted ? t("ready") : t("accessibilityHelp")}><div className="inline-actions"><button className="secondary-button" onClick={async () => { try { await invoke("open_accessibility_settings"); } catch (reason) { onToast(errorMessage(reason, language), "error"); } }}>{t("grant")}</button><button className="secondary-button" disabled={accessibilityBusy} onClick={async () => { setAccessibilityBusy(true); try { await invoke("refresh_accessibility_status"); await onRefresh(); } catch (reason) { onToast(errorMessage(reason, language), "error"); } finally { setAccessibilityBusy(false); } }}>{accessibilityBusy ? <LoaderCircle className="spin" /> : data.accessibilityGranted ? <Check /> : <RefreshCw />}{t("refresh")}</button></div></SettingRow>
      <SettingRow title={t("shortcut")} detail={formatShortcut(draft.dictationShortcut)}><button className="secondary-button" disabled={isBusy} onClick={async () => { if (shortcutBusy) { try { await invoke("cancel_shortcut_capture"); } catch (reason) { onToast(errorMessage(reason, language), "error"); } finally { setShortcutBusy(false); } return; } setShortcutBusy(true); try { await invoke("begin_shortcut_capture"); } catch (reason) { setShortcutBusy(false); onToast(errorMessage(reason, language), "error"); } }}>{shortcutBusy ? <X /> : null}{shortcutBusy ? t("cancel") : t("changeShortcut")}</button></SettingRow>
      <SettingRow title={t("autoPaste")} detail={t("autoPasteHelp")}><Toggle label={t("autoPaste")} checked={draft.autoPaste} onChange={(autoPaste) => setDraft({ ...draft, autoPaste })} /></SettingRow>
    </SettingsSection>
    <SettingsSection icon={<FolderOpen />} title={t("storage")}>
      <StorageControls settings={draft} language={language} outputPath={draft.outputDirectory ?? data.defaultOutputDirectory} onChange={setDraft} onChooseFolder={chooseFolder} />
    </SettingsSection>
    <SettingsSection icon={<RefreshCw />} title={t("updates")}>
      <SettingRow title={t("launchAtLogin")}><Toggle label={t("launchAtLogin")} checked={draft.launchAtLogin} onChange={(launchAtLogin) => setDraft({ ...draft, launchAtLogin })} /></SettingRow>
      <div className="update-row"><button className="secondary-button" onClick={onCheckUpdates}><RefreshCw />{t("checkUpdates")}</button>{updateMessage && <span>{updateMessage}</span>}{availableUpdate && <button className="primary-button" disabled={isBusy} title={isBusy ? t("updateBusy") : undefined} onClick={onInstallUpdate}>{t("installUpdate")}</button>}</div>
    </SettingsSection>
    <SettingsSection icon={<ShieldCheck />} title={t("diagnostics")}>
      <p className="section-help">{t("diagnosticsHelp")}</p><div className="inline-actions"><button className="secondary-button" disabled={diagnosticBusy} onClick={async () => { setDiagnosticBusy(true); try { const path = await invoke<string>("create_diagnostic_bundle"); await openPath(path); } catch (reason) { onToast(errorMessage(reason, language), "error"); } finally { setDiagnosticBusy(false); } }}>{diagnosticBusy ? <LoaderCircle className="spin" /> : <FileText />}{t("createDiagnostics")}</button><button className="secondary-button" onClick={async () => { try { await openUrl("https://github.com/bkafelov-aidoo/Aidoo-Whisper/issues"); } catch (reason) { onToast(errorMessage(reason, language), "error"); } }}><ExternalLink />{t("openSupport")}</button></div>
    </SettingsSection>
    <footer className="settings-footer"><button className="primary-button large" disabled={isBusy} title={isBusy ? t("finishDictationFirst") : undefined} onClick={() => onSave(draft)}><Check />{t("save")}</button></footer>
  </div>;
}

function SettingsSection({ icon, title, children }: { icon: React.ReactNode; title: string; children: React.ReactNode }) {
  return <section className="settings-section"><header><div>{icon}</div><h2>{title}</h2></header><div className="settings-body">{children}</div></section>;
}

function SettingRow({ title, detail, children }: { title: string; detail?: string; children: React.ReactNode }) {
  return <div className="setting-row"><div><strong>{title}</strong>{detail && <span>{detail}</span>}</div>{children}</div>;
}

function Toggle({ label, checked, onChange }: { label: string; checked: boolean; onChange: (value: boolean) => void }) {
  return <button type="button" role="switch" aria-label={label} aria-checked={checked} className={`toggle ${checked ? "on" : ""}`} onClick={() => onChange(!checked)}><span /></button>;
}

function ModelPicker({ settings, language, onChange }: { settings: AppSettings; language: AppLanguage; onChange: (settings: AppSettings) => void }) {
  const t = translator(language);
  return <div className="model-language-grid"><button type="button" aria-pressed={settings.model === "gpt-4o-mini-transcribe"} className={`model-option ${settings.model === "gpt-4o-mini-transcribe" ? "selected" : ""}`} onClick={() => onChange({ ...settings, model: "gpt-4o-mini-transcribe" })}><span><Sparkles /></span><strong>{t("economy")}</strong><small>{t("economyHelp")}</small><em>$0.003 / min</em></button><button type="button" aria-pressed={settings.model === "gpt-transcribe"} className={`model-option ${settings.model === "gpt-transcribe" ? "selected" : ""}`} onClick={() => onChange({ ...settings, model: "gpt-transcribe" })}><span><ShieldCheck /></span><strong>{t("accuracy")}</strong><small>{t("accuracyHelp")}</small><em>$0.0045 / min</em></button><label className="language-select"><span>{t("modelLanguage")}</span><select value={settings.language} onChange={(event) => onChange({ ...settings, language: event.target.value })}><option value="auto">{t("autoLanguage")}</option><option value="bg">{t("bulgarian")}</option><option value="en">{t("english")}</option><option value="de">Deutsch</option><option value="es">Español</option><option value="fr">Français</option><option value="it">Italiano</option></select></label></div>;
}

function StorageControls({ settings, language, outputPath, onChange, onChooseFolder }: { settings: AppSettings; language: AppLanguage; outputPath: string; onChange: (settings: AppSettings) => void; onChooseFolder: () => void }) {
  const t = translator(language);
  return <div className="storage-controls"><SettingRow title={t("saveFlac")} detail={t("saveFlacHelp")}><Toggle label={t("saveFlac")} checked={settings.saveAudio} onChange={(saveAudio) => onChange({ ...settings, saveAudio })} /></SettingRow><SettingRow title={t("saveTxt")} detail={t("saveTxtHelp")}><Toggle label={t("saveTxt")} checked={settings.saveText} onChange={(saveText) => onChange({ ...settings, saveText })} /></SettingRow><SettingRow title={t("saveHistory")} detail={t("saveHistoryHelp")}><Toggle label={t("saveHistory")} checked={settings.historyEnabled} onChange={(historyEnabled) => onChange({ ...settings, historyEnabled })} /></SettingRow><div className="folder-picker"><div><strong>{t("folder")}</strong><span title={outputPath}>{outputPath}</span></div><button className="secondary-button" onClick={onChooseFolder}><FolderOpen />{t("chooseFolder")}</button></div></div>;
}

function Onboarding({ data, language, onData, onPersist, onRefresh, onClose, onToast }: { data: BootstrapState; language: AppLanguage; onData: React.Dispatch<React.SetStateAction<BootstrapState | null>>; onPersist: (settings: AppSettings) => Promise<AppSettings>; onRefresh: () => Promise<BootstrapState>; onClose: () => void; onToast: ToastHandler }) {
  const t = translator(language);
  const [step, setStep] = useState(0);
  const [draft, setDraft] = useState(data.settings);
  const [apiKey, setApiKey] = useState("");
  const [busy, setBusy] = useState(false);
  const [micTested, setMicTested] = useState(data.settings.onboardingComplete);
  const [shortcutBusy, setShortcutBusy] = useState(false);
  useShortcutCapture(shortcutBusy, setShortcutBusy, (binding) => setDraft((current) => ({ ...current, dictationShortcut: binding })), (message) => onToast(errorMessage(message, language), "error"));
  const steps = [t("stepAccount"), t("stepPermissions"), t("stepShortcut"), t("stepPreferences"), t("stepStorage"), t("stepReady")];

  const saveDraft = async () => { const saved = await onPersist(draft); setDraft(saved); return saved; };
  const next = async () => { try { await saveDraft(); setStep((value) => Math.min(steps.length - 1, value + 1)); } catch (reason) { onToast(errorMessage(reason, language), "error"); } };
  const chooseFolder = async () => {
    try {
      const selected = await open({ directory: true, multiple: false, defaultPath: draft.outputDirectory ?? data.defaultOutputDirectory });
      if (selected) setDraft({ ...draft, outputDirectory: selected });
    } catch (reason) {
      onToast(errorMessage(reason, language), "error");
    }
  };
  const missing = !data.hasApiKey ? t("keyRequired") : !micTested ? t("micRequired") : !data.accessibilityGranted ? t("accessRequired") : null;

  return <div className="modal-backdrop"><section className="onboarding-modal"><header className="onboarding-header"><div className="brand compact"><img src="/app-icon.png" alt="" /><div><strong>AIDOO</strong><span>Whisper Lite</span></div></div><button className="close-button" aria-label={t("closeContinueLater")} onClick={onClose}><X /></button></header><div className="stepper">{steps.map((name, index) => <div key={name} className={`${index === step ? "active" : ""} ${index < step ? "done" : ""}`}><i>{index < step ? <Check /> : index + 1}</i><span>{name}</span></div>)}</div><div className="onboarding-content">
    {step === 0 && <div className="onboarding-step"><span className="step-icon"><KeyRound /></span><h1>{t("welcomeTitle")}</h1><p>{t("welcomeBody")}</p><div className="instruction-card"><CircleHelp /><span>{t("apiSteps")}</span></div><button className="secondary-button wide" onClick={async () => { try { await openUrl("https://platform.openai.com/api-keys"); } catch (reason) { onToast(errorMessage(reason, language), "error"); } }}><ExternalLink />{t("createKey")}</button><div className="api-input"><input type="password" placeholder="sk-…" value={apiKey} onChange={(event) => setApiKey(event.target.value)} /><button className="primary-button" disabled={busy || !apiKey.trim()} onClick={async () => { setBusy(true); try { await invoke("save_api_key", { apiKey }); setApiKey(""); const nextData = await onRefresh(); onData(nextData); onToast(t("keySaved")); } catch (reason) { onToast(errorMessage(reason, language), "error"); } finally { setBusy(false); } }}>{busy ? <LoaderCircle className="spin" /> : <ShieldCheck />}{t("verifySave")}</button></div>{data.hasApiKey && <div className="success-note"><Check />{t("keySaved")}</div>}</div>}
    {step === 1 && <div className="onboarding-step"><span className="step-icon"><Mic /></span><h1>{t("stepPermissions")}</h1><p>{t("accessibilityHelp")}</p><div className="permission-list"><article className={micTested ? "ready" : ""}><div><Mic /></div><span><strong>{t("microphone")}</strong><small>{draft.microphoneName ?? data.microphones[0] ?? t("micRequired")} · {t("microphoneTestHelp")}</small></span><select value={draft.microphoneName ?? ""} onChange={(event) => { setMicTested(false); setDraft({ ...draft, microphoneName: event.target.value || null }); }}><option value="">{t("systemDefault")}</option>{data.microphones.map((item) => <option key={item} value={item}>{item}</option>)}</select><button className="secondary-button" disabled={busy || !data.microphones.length} onClick={async () => { setBusy(true); try { const probe = await invoke<MicrophoneProbe>("test_microphone", { microphoneName: draft.microphoneName, automaticFallback: draft.automaticMicrophoneFallback }); if (!probe.heardAudio) throw new Error(t("microphoneSilent")); setMicTested(true); onToast(probe.usedFallback ? t("microphoneFallback", { name: probe.deviceName }) : t("microphoneOk"), probe.usedFallback ? "warning" : "success"); } catch (reason) { setMicTested(false); onToast(errorMessage(reason, language), "error"); } finally { setBusy(false); } }}>{busy ? <LoaderCircle className="spin" /> : <AudioLines />}{t("testMicrophone")}</button></article><article className={data.accessibilityGranted ? "ready" : ""}><div><ShieldCheck /></div><span><strong>{t("accessibility")}</strong><small>{t("accessibilityHelp")}</small></span><button className="secondary-button" onClick={async () => { try { await invoke("open_accessibility_settings"); } catch (reason) { onToast(errorMessage(reason, language), "error"); } }}>{t("grant")}</button><button className="secondary-button" disabled={busy} onClick={async () => { setBusy(true); try { const granted = await invoke<boolean>("refresh_accessibility_status"); onData((current) => current ? { ...current, accessibilityGranted: granted } : current); } catch (reason) { onToast(errorMessage(reason, language), "error"); } finally { setBusy(false); } }}>{data.accessibilityGranted ? <Check /> : <RefreshCw />}{t("refresh")}</button></article></div><SettingRow title={t("automaticMicrophoneFallback")} detail={t("automaticMicrophoneFallbackHelp")}><Toggle label={t("automaticMicrophoneFallback")} checked={draft.automaticMicrophoneFallback} onChange={(automaticMicrophoneFallback) => setDraft({ ...draft, automaticMicrophoneFallback })} /></SettingRow></div>}
    {step === 2 && <div className="onboarding-step"><span className="step-icon"><AudioLines /></span><h1>{t("shortcut")}</h1><p>{t("holdShortcut", { shortcut: formatShortcut(draft.dictationShortcut) })}</p><button className={`shortcut-capture ${shortcutBusy ? "listening" : ""}`} onClick={async () => { setShortcutBusy(true); try { await invoke("begin_shortcut_capture"); } catch (reason) { setShortcutBusy(false); onToast(errorMessage(reason, language), "error"); } }}>{shortcutBusy ? <><AudioLines />{t("pressShortcut")}</> : <><kbd>{formatShortcut(draft.dictationShortcut)}</kbd>{t("changeShortcut")}</>}</button>{shortcutBusy && <button className="text-button" onClick={async () => { try { await invoke("cancel_shortcut_capture"); } catch (reason) { onToast(errorMessage(reason, language), "error"); } finally { setShortcutBusy(false); } }}>{t("cancel")}</button>}</div>}
    {step === 3 && <div className="onboarding-step"><span className="step-icon"><Languages /></span><h1>{t("modelLanguage")}</h1><p>{language === "bg" ? "Изберете баланс между цена и точност и задайте език, за да избегнете автоматичното разпознаване." : "Choose your cost/accuracy balance and set a language to skip automatic detection."}</p><ModelPicker settings={draft} language={language} onChange={setDraft} /></div>}
    {step === 4 && <div className="onboarding-step"><span className="step-icon"><FolderOpen /></span><h1>{t("storage")}</h1><p>{language === "bg" ? "Всеки тип съхранение се управлява отделно. Потребителските файлове никога не се изтриват автоматично." : "Each storage type is controlled separately. User files are never deleted automatically."}</p><StorageControls settings={draft} language={language} outputPath={draft.outputDirectory ?? data.defaultOutputDirectory} onChange={setDraft} onChooseFolder={chooseFolder} /></div>}
    {step === 5 && <div className="onboarding-step ready-step"><span className={`step-icon ${missing ? "warning" : "success"}`}>{missing ? <AlertCircle /> : <Check />}</span><h1>{missing ?? t("allReady")}</h1><p>{missing ? t("onboardingIncomplete") : t("readyBody")}</p><div className="ready-summary"><span><KeyRound /><strong>OpenAI</strong><em>{data.hasApiKey ? "✓" : "—"}</em></span><span><Mic /><strong>{t("microphone")}</strong><em>{micTested ? "✓" : "—"}</em></span><span><ShieldCheck /><strong>Accessibility</strong><em>{data.accessibilityGranted ? "✓" : "—"}</em></span><span><AudioLines /><strong>{t("shortcut")}</strong><em>{formatShortcut(draft.dictationShortcut)}</em></span></div></div>}
  </div><footer className="onboarding-footer"><button className="text-button" onClick={onClose}>{t("closeContinueLater")}</button><div>{step > 0 && <button className="secondary-button" onClick={() => setStep((value) => value - 1)}><ChevronLeft />{t("back")}</button>}{step < steps.length - 1 ? <button className="primary-button" onClick={next}>{t("continue")}<ChevronRight /></button> : <button className="primary-button" disabled={Boolean(missing)} onClick={async () => { try { const completed = { ...draft, onboardingComplete: true }; await onPersist(completed); await onRefresh(); onClose(); } catch (reason) { onToast(errorMessage(reason, language), "error"); } }}><Check />{t("finish")}</button>}</div></footer></section></div>;
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
  return <div className="modal-backdrop small"><section className="confirm-dialog"><button className="close-button" aria-label={t("cancel")} onClick={onCancel}><X /></button><span className="danger-icon"><Trash2 /></span><h2>{t("deleteQuestion")}</h2><p>{entry.text}</p><button className="secondary-button" onClick={() => onDelete(false)}>{t("historyOnly")}</button><button className="danger-button" onClick={() => onDelete(true)}>{t("historyAndFiles")}</button></section></div>;
}

function StatusPill({ ready, label }: { ready: boolean; label: string }) {
  return <span className={`status-pill ${ready ? "ready" : "attention"}`}><i />{label}</span>;
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
  return new Intl.DateTimeFormat(language === "bg" ? "bg-BG" : "en-US", { dateStyle: "medium", timeStyle: "short" }).format(new Date(value));
}

function modelLabel(model: string, language: AppLanguage) {
  return model === "gpt-4o-mini-transcribe" ? (language === "bg" ? "Икономичен" : "Economy") : (language === "bg" ? "Максимална точност" : "Maximum accuracy");
}
