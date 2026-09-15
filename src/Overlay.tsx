import { useEffect, useMemo, useRef, useState } from "react";
import { invoke } from "@tauri-apps/api/core";
import { listen } from "@tauri-apps/api/event";
import { getCurrentWindow, LogicalSize } from "@tauri-apps/api/window";
import { Check, CircleAlert, LoaderCircle, Mic, Square } from "lucide-react";
import type { AppSettings, OverlayBootstrapState, RecordingProgress, RecordingSnapshot } from "./types";
import { errorMessage, progressLabel, resolveLanguage } from "./i18n";

const initial: RecordingSnapshot = {
  state: "idle",
  progress: { percent: 0, stage: "", determinate: false },
  elapsedSeconds: 0,
  error: null,
};

const stateText = {
  bg: {
    idle: "Готов за диктовка",
    starting: "Стартирам микрофона…",
    recording: "Слушам ви",
    transcribing: "Транскрибирам…",
    done: "Готово за нов запис",
    complete: "Текстът е транскрибиран и копиран.",
    error: "Възникна грешка",
    release: "Отпуснете shortcut-а за край",
    stop: "Стоп",
    stopTitle: "Спри записа и започни транскрипцията",
  },
  en: {
    idle: "Ready for dictation",
    starting: "Starting the microphone…",
    recording: "Listening",
    transcribing: "Transcribing…",
    done: "Ready for a new recording",
    complete: "The text is transcribed and copied.",
    error: "Something went wrong",
    release: "Release the shortcut to finish",
    stop: "Stop",
    stopTitle: "Stop recording and start transcription",
  },
} as const;

export default function Overlay() {
  const [snapshot, setSnapshot] = useState(initial);
  const [language, setLanguage] = useState<"bg" | "en">("bg");
  const [notice, setNotice] = useState<string | null>(null);
  const card = useRef<HTMLDivElement>(null);

  useEffect(() => {
    let disposed = false;
    const unlisten: Array<() => void> = [];
    void invoke<OverlayBootstrapState>("overlay_bootstrap").then((state) => {
      if (!disposed) {
        setSnapshot(state.recording);
        setLanguage(resolveLanguage(state.uiLanguage));
      }
    });
    void listen<RecordingSnapshot>("recording:snapshot", ({ payload }) => setSnapshot(payload)).then((fn) => unlisten.push(fn));
    void listen<string>("recording:state", ({ payload }) => {
      if (payload === "starting" || payload === "idle") setNotice(null);
      setSnapshot((current) => ({ ...current, state: payload as RecordingSnapshot["state"] }));
    }).then((fn) => unlisten.push(fn));
    void listen<RecordingProgress>("recording:progress", ({ payload }) => setSnapshot((current) => ({ ...current, progress: payload }))).then((fn) => unlisten.push(fn));
    void listen<string>("recording:error", ({ payload }) => setSnapshot((current) => ({ ...current, state: "error", error: payload }))).then((fn) => unlisten.push(fn));
    void listen<AppSettings>("settings:changed", ({ payload }) => setLanguage(resolveLanguage(payload.uiLanguage))).then((fn) => unlisten.push(fn));
    void listen<string>("toast", ({ payload }) => setNotice(payload)).then((fn) => unlisten.push(fn));
    return () => {
      disposed = true;
      unlisten.forEach((fn) => fn());
    };
  }, []);

  useEffect(() => {
    if (snapshot.state !== "recording") return;
    const timer = window.setInterval(() => {
      setSnapshot((current) => current.state === "recording" ? { ...current, elapsedSeconds: current.elapsedSeconds + 0.1 } : current);
    }, 100);
    return () => window.clearInterval(timer);
  }, [snapshot.state]);

  useEffect(() => {
    let disposed = false;
    let inFlight = false;
    const sync = async () => {
      if (disposed || inFlight) return;
      inFlight = true;
      try {
        const current = await invoke<RecordingSnapshot>("current_recording_snapshot");
        if (!disposed) setSnapshot(current);
      } catch {
        // Native events remain the primary path; the poll only repairs missed wake-up events.
      } finally {
        inFlight = false;
      }
    };
    const onVisibilityChange = () => { if (!document.hidden) void sync(); };
    document.addEventListener("visibilitychange", onVisibilityChange);
    void sync();
    const timer = window.setInterval(() => void sync(), snapshot.state === "idle" ? 5000 : 150);
    return () => {
      disposed = true;
      window.clearInterval(timer);
      document.removeEventListener("visibilitychange", onVisibilityChange);
    };
  }, [snapshot.state]);

  useEffect(() => {
    const height = Math.max(132, Math.min(260, (card.current?.scrollHeight ?? 84) + 48));
    void getCurrentWindow().setSize(new LogicalSize(552, height)).then(() => invoke("reposition_overlay"));
  }, [snapshot.state, snapshot.error, snapshot.progress.stage, notice]);

  const label = stateText[language];
  const stage = progressLabel(snapshot.progress.stage, language);
  const stopRecording = async () => {
    try {
      await invoke("stop_and_transcribe");
    } catch (reason) {
      setNotice(errorMessage(reason, language));
    }
  };
  const icon = useMemo(() => {
    if (snapshot.state === "done") return <Check />;
    if (snapshot.state === "error") return <CircleAlert />;
    if (snapshot.state === "starting" || snapshot.state === "transcribing") return <LoaderCircle className="spin" />;
    return <Mic />;
  }, [snapshot.state]);

  return (
    <main className="overlay-shell">
      <div ref={card} className={`overlay-card ${snapshot.state}`}>
        <div className={`overlay-state-icon ${snapshot.state}`} aria-hidden="true">{icon}</div>
        <div className="overlay-copy" role={snapshot.state === "error" ? "alert" : "status"} aria-live={snapshot.state === "error" ? "assertive" : "polite"} aria-atomic="true">
          <strong>{label[snapshot.state]}</strong>
          {snapshot.state === "recording" && <span>{label.release}</span>}
          {snapshot.state === "transcribing" && <span>{stage}</span>}
          {snapshot.state === "error" && <span className="error-text">{errorMessage(snapshot.error, language)}</span>}
          {snapshot.state === "starting" && <span>{stage}</span>}
          {snapshot.state === "done" && <span>{label.complete}</span>}
          {notice && snapshot.state !== "error" && <span className="notice-text">{errorMessage(notice, language)}</span>}
        </div>
        {snapshot.state === "recording" && (
          <div className="overlay-live">
            <div className="overlay-wave" aria-hidden="true">
              {Array.from({ length: 9 }, (_, index) => <i key={index} style={{ animationDelay: `${index * -0.09}s` }} />)}
            </div>
            <time className="overlay-time">{formatDuration(snapshot.elapsedSeconds)}</time>
            <button className="overlay-stop" type="button" title={label.stopTitle} aria-label={label.stopTitle} onClick={() => void stopRecording()}>
              <Square aria-hidden="true" />
              <span>{label.stop}</span>
            </button>
          </div>
        )}
        {snapshot.state === "transcribing" && (
          <div className={`overlay-progress ${snapshot.progress.determinate ? "" : "indeterminate"}`} role="progressbar" aria-label={stage} aria-valuemin={snapshot.progress.determinate ? 0 : undefined} aria-valuemax={snapshot.progress.determinate ? 100 : undefined} aria-valuenow={snapshot.progress.determinate ? snapshot.progress.percent : undefined}>
            <i style={{ width: snapshot.progress.determinate ? `${snapshot.progress.percent}%` : "38%" }} />
          </div>
        )}
      </div>
    </main>
  );
}

function formatDuration(seconds: number) {
  const total = Math.max(0, Math.floor(seconds));
  return `${String(Math.floor(total / 60)).padStart(2, "0")}:${String(total % 60).padStart(2, "0")}`;
}
