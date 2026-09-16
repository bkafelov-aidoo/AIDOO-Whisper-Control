import { useCallback, useEffect, useRef, useState } from "react";
import { invoke } from "@tauri-apps/api/core";
import { listen } from "@tauri-apps/api/event";
import { createEventScope } from "./lib/event-scope";
import { disable, enable, isEnabled } from "@tauri-apps/plugin-autostart";
import { AlertCircle, Check, CircleDollarSign, History, LoaderCircle, MessageCircle, Mic, RefreshCw, Settings } from "lucide-react";
import { errorMessage, resolveLanguage, translator } from "./i18n";
import { type AppLanguage, type AppSettings, type BootstrapState, type FailedRecording, type RecordingSnapshot, type TranscriptEntry, type TranscriptionCompleted, type UsageLedger } from "./types";
import { type ToastHandler } from "./ui-types";
import { appStatus, formatShortcut } from "./lib/presentation";
import { Dashboard } from "./pages/Dashboard";
import { HistoryPage } from "./pages/HistoryPage";
import { SettingsPage } from "./pages/SettingsPage";
import { AssistantPage } from "./pages/AssistantPage";
import { UsagePage } from "./pages/UsagePage";
import { Onboarding } from "./components/Onboarding";
import { DeleteDialog } from "./components/DeleteDialog";
import type { ToastTone } from "./ui-types";
import { useLiveConversation } from "./hooks/useLiveConversation";

type Page = "dictation" | "assistant" | "history" | "usage" | "settings";

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

  const showLiveError = useCallback((reason: unknown) => {
    showToast(errorMessage(reason, languageRef.current), "error");
  }, [showToast]);
  const handleAssistantDictation = useCallback(() => {
    setPage("dictation");
    showToast(translator(languageRef.current)("assistantDictationStarted"));
  }, [showToast]);
  const handleAssistantRequested = useCallback(() => setPage("assistant"), []);
  const live = useLiveConversation(data?.settings.microphoneName ?? null, showLiveError, handleAssistantDictation, handleAssistantRequested);

  const refresh = useCallback(async () => {
    try {
      const nativeState = await invoke<BootstrapState>("bootstrap");
      let next = nativeState;
      try {
        const launchAtLogin = await isEnabled();
        if (launchAtLogin !== nativeState.settings.launchAtLogin) {
          next = { ...nativeState, settings: { ...nativeState.settings, launchAtLogin } };
        }
      } catch { /* Keep the last persisted value when macOS cannot report Login Item state. */ }
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
    const events = createEventScope(listen, (reason) => showToast(errorMessage(reason, languageRef.current), "error"));
    events.listen<TranscriptionCompleted>("transcription:completed", ({ payload }) => {
      setData((current) => {
        if (!current || !payload.entry) return current;
        return { ...current, history: [payload.entry, ...current.history.filter((item) => item.id !== payload.entry!.id)].slice(0, 10) };
      });
    });
    events.listen<UsageLedger>("usage:changed", ({ payload }) => {
      setData((current) => current ? { ...current, usage: payload } : current);
    });
    events.listen<string>("toast", ({ payload }) => showToast(errorMessage(payload, languageRef.current), "warning"));
    events.listen<boolean>("aidoo:connection-changed", () => {
      void refresh().catch(() => undefined);
    });
    events.listen<FailedRecording | null>("failed-recording:changed", ({ payload }) => {
      setData((current) => current ? { ...current, failedRecording: payload } : current);
    });
    events.listen<string>("recording:state", ({ payload }) => {
      setData((current) => current ? { ...current, recording: { ...current.recording, state: payload as BootstrapState["recording"]["state"] } } : current);
    });
    events.listen<RecordingSnapshot>("recording:snapshot", ({ payload }) => {
      setData((current) => current ? { ...current, recording: payload } : current);
    });
    events.listen<Page>("navigate", ({ payload }) => setPage(payload));
    return () => {
      events.dispose();
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
  const recordingBusy = ["starting", "recording", "transcribing"].includes(data.recording.state);
  const liveBusy = !["idle", "error"].includes(live.phase);
  const isBusy = recordingBusy || liveBusy;
  const status = appStatus(data, language);

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
          <NavButton active={page === "assistant"} disabled={recordingBusy} icon={<MessageCircle />} label={t("assistant")} onClick={() => setPage("assistant")} />
          <NavButton active={page === "history"} disabled={isBusy} icon={<History />} label={t("history")} badge={data.history.length || undefined} onClick={() => setPage("history")} />
          <NavButton active={page === "usage"} disabled={isBusy} icon={<CircleDollarSign />} label={t("usage")} onClick={() => setPage("usage")} />
          <NavButton active={page === "settings"} disabled={isBusy} icon={<Settings />} label={t("settings")} onClick={() => setPage("settings")} />
        </nav>
        <div className={`sidebar-status ${status.tone}`}>
          <i />
          <span>{liveBusy ? (live.phase === "speaking" ? t("liveSpeaking") : live.phase === "listening" ? t("liveListening") : t("liveConnecting")) : status.label}</span>
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
        {page === "assistant" && (
          <AssistantPage
            live={live}
            language={language}
            available={data.settings.onboardingComplete && data.hasApiKey}
            dictationBusy={recordingBusy}
            aidooConnected={data.aidooConnected}
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
        {page === "usage" && <UsagePage usage={data.usage} language={language} />}
        {page === "settings" && (
          <SettingsPage
            data={data}
            language={language}
            isBusy={isBusy}
            onSave={async (settings) => {
              try {
                const previousLaunchAtLogin = await isEnabled();
                const launchAtLoginChanged = previousLaunchAtLogin !== settings.launchAtLogin;
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
              } catch (reason) {
                showToast(errorMessage(reason, language), "error");
                throw reason;
              }
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

function settingsMatch(left: AppSettings, right: AppSettings) {
  return JSON.stringify(left) === JSON.stringify(right);
}
