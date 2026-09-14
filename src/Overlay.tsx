import { useEffect, useMemo, useRef, useState } from "react";
import { invoke } from "@tauri-apps/api/core";
import { listen } from "@tauri-apps/api/event";
import { getCurrentWindow, LogicalSize } from "@tauri-apps/api/window";
import { Check, CircleAlert, LoaderCircle, Mic } from "lucide-react";
import type { BootstrapState, RecordingProgress, RecordingSnapshot } from "./types";
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
    done: "Текстът е готов",
    error: "Възникна грешка",
    release: "Отпуснете shortcut-а за край",
  },
  en: {
    idle: "Ready for dictation",
    starting: "Starting the microphone…",
    recording: "Listening",
    transcribing: "Transcribing…",
    done: "Text is ready",
    error: "Something went wrong",
    release: "Release the shortcut to finish",
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
    void invoke<BootstrapState>("bootstrap").then((state) => {
      if (!disposed) {
        setSnapshot(state.recording);
        setLanguage(resolveLanguage(state.settings.uiLanguage));
      }
    });
    void listen<RecordingSnapshot>("recording:snapshot", ({ payload }) => setSnapshot(payload)).then((fn) => unlisten.push(fn));
    void listen<string>("recording:state", ({ payload }) => {
      if (payload === "starting" || payload === "idle") setNotice(null);
      setSnapshot((current) => ({ ...current, state: payload as RecordingSnapshot["state"] }));
    }).then((fn) => unlisten.push(fn));
    void listen<RecordingProgress>("recording:progress", ({ payload }) => setSnapshot((current) => ({ ...current, progress: payload }))).then((fn) => unlisten.push(fn));
    void listen<string>("recording:error", ({ payload }) => setSnapshot((current) => ({ ...current, state: "error", error: payload }))).then((fn) => unlisten.push(fn));
    void listen<BootstrapState["settings"]>("settings:changed", ({ payload }) => setLanguage(resolveLanguage(payload.uiLanguage))).then((fn) => unlisten.push(fn));
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
    const height = Math.max(132, Math.min(260, (card.current?.scrollHeight ?? 84) + 48));
    void getCurrentWindow().setSize(new LogicalSize(552, height)).then(() => invoke("reposition_overlay"));
  }, [snapshot.state, snapshot.error, snapshot.progress.stage, notice]);

  const label = stateText[language];
  const stage = progressLabel(snapshot.progress.stage, language);
  const icon = useMemo(() => {
    if (snapshot.state === "done") return <Check />;
    if (snapshot.state === "error") return <CircleAlert />;
    if (snapshot.state === "starting" || snapshot.state === "transcribing") return <LoaderCircle className="spin" />;
    return <Mic />;
  }, [snapshot.state]);

  return (
    <main className="overlay-shell">
      <div ref={card} className={`overlay-card ${snapshot.state}`}>
        <div className={`overlay-state-icon ${snapshot.state}`}>{icon}</div>
        <div className="overlay-copy">
          <strong>{label[snapshot.state]}</strong>
          {snapshot.state === "recording" && <span>{label.release}</span>}
          {snapshot.state === "transcribing" && <span>{stage}</span>}
          {snapshot.state === "error" && <span className="error-text">{errorMessage(snapshot.error, language)}</span>}
          {snapshot.state === "starting" && <span>{stage}</span>}
          {notice && snapshot.state !== "error" && <span className="notice-text">{errorMessage(notice, language)}</span>}
        </div>
        {snapshot.state === "recording" && (
          <div className="overlay-live">
            <div className="overlay-wave" aria-hidden="true">
              {Array.from({ length: 9 }, (_, index) => <i key={index} style={{ animationDelay: `${index * -0.09}s` }} />)}
            </div>
            <time className="overlay-time">{formatDuration(snapshot.elapsedSeconds)}</time>
          </div>
        )}
        {snapshot.state === "transcribing" && (
          <div className={`overlay-progress ${snapshot.progress.determinate ? "" : "indeterminate"}`} aria-label={`${snapshot.progress.percent}%`}>
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
