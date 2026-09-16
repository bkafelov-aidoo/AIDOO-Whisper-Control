import { useEffect, useRef, useState } from "react";
import { invoke } from "@tauri-apps/api/core";
import { listen } from "@tauri-apps/api/event";
import { AudioLines, Check, LoaderCircle, Mic, RotateCcw, X } from "lucide-react";
import { createEventScope } from "../lib/event-scope";
import { translator } from "../i18n";
import type { AppLanguage, WakeWordCalibrationScore } from "../types";
import type { ToastHandler } from "../ui-types";
import { useDialogFocus } from "../hooks/useDialogFocus";

type Phase = "intro" | "starting" | "countdown" | "listening" | "success" | "failed";
const REQUIRED_MATCHES = 3;
const INITIAL_COUNTDOWN = 2;
const NEXT_ATTEMPT_DELAY_MS = 900;

export function WakeWordCalibrationDialog({ language, onClose, onToast }: {
  language: AppLanguage;
  onClose: () => void;
  onToast: ToastHandler;
}) {
  const t = translator(language);
  const [phase, setPhase] = useState<Phase>("intro");
  const [countdown, setCountdown] = useState(INITIAL_COUNTDOWN);
  const [matches, setMatches] = useState(0);
  const [level, setLevel] = useState(0);
  const [hearing, setHearing] = useState(false);
  const [heardVoice, setHeardVoice] = useState(false);
  const [recognized, setRecognized] = useState(false);
  const phaseRef = useRef<Phase>("intro");
  const acceptingRef = useRef(false);
  const timeoutRef = useRef<number | null>(null);
  const hearingTimerRef = useRef<number | null>(null);
  const countdownTimerRef = useRef<number | null>(null);
  const nextAttemptTimerRef = useRef<number | null>(null);

  const moveTo = (next: Phase) => {
    phaseRef.current = next;
    setPhase(next);
  };
  const stop = async () => {
    if (timeoutRef.current) window.clearTimeout(timeoutRef.current);
    timeoutRef.current = null;
    if (hearingTimerRef.current) window.clearTimeout(hearingTimerRef.current);
    hearingTimerRef.current = null;
    if (countdownTimerRef.current) window.clearInterval(countdownTimerRef.current);
    countdownTimerRef.current = null;
    if (nextAttemptTimerRef.current) window.clearTimeout(nextAttemptTimerRef.current);
    nextAttemptTimerRef.current = null;
    acceptingRef.current = false;
    setHearing(false);
    await invoke("stop_wake_word_calibration").catch(() => undefined);
  };
  const close = async () => {
    await stop();
    onClose();
  };
  const dialogRef = useDialogFocus(() => void close(), phase !== "starting");

  useEffect(() => {
    const events = createEventScope(listen, (reason) => onToast(String(reason), "error"));
    events.listen<number>("wake-word:calibration-level", ({ payload }) => {
      if (phaseRef.current !== "countdown" && phaseRef.current !== "listening") return;
      setLevel(Math.min(1, payload / 0.05));
      if (payload < 0.006 || phaseRef.current !== "listening" || !acceptingRef.current) return;
      setHearing(true);
      setHeardVoice(true);
      if (hearingTimerRef.current) window.clearTimeout(hearingTimerRef.current);
      hearingTimerRef.current = window.setTimeout(() => setHearing(false), 650);
    });
    events.listen<WakeWordCalibrationScore>("wake-word:calibration-score", ({ payload }) => {
      if (phaseRef.current === "countdown" || phaseRef.current === "listening") {
        setLevel(Math.min(1, payload.rms / 0.05));
      }
    });
    events.listen<number>("wake-word:calibration-detected", () => {
      if (phaseRef.current !== "listening" || !acceptingRef.current) return;
      acceptingRef.current = false;
      setRecognized(true);
      setHearing(false);
      setHeardVoice(false);
      setLevel(1);
      if (hearingTimerRef.current) window.clearTimeout(hearingTimerRef.current);
      setMatches((current) => {
        const next = Math.min(REQUIRED_MATCHES, current + 1);
        if (next === REQUIRED_MATCHES) {
          nextAttemptTimerRef.current = window.setTimeout(() => {
            moveTo("success");
            void stop();
          }, 450);
        } else {
          nextAttemptTimerRef.current = window.setTimeout(() => {
            setRecognized(false);
            setLevel(0);
            acceptingRef.current = true;
          }, NEXT_ATTEMPT_DELAY_MS);
        }
        return next;
      });
    });
    events.listen<string>("wake-word:calibration-error", ({ payload }) => {
      moveTo("failed");
      onToast(payload, "error");
    });
    return () => {
      events.dispose();
      void stop();
    };
  }, [onToast]);

  const start = async () => {
    setMatches(0);
    setLevel(0);
    setHearing(false);
    setHeardVoice(false);
    setRecognized(false);
    acceptingRef.current = false;
    setCountdown(INITIAL_COUNTDOWN);
    moveTo("starting");
    try {
      await invoke("start_wake_word_calibration");
      moveTo("countdown");
      let value = INITIAL_COUNTDOWN;
      countdownTimerRef.current = window.setInterval(() => {
        value -= 1;
        if (value > 0) {
          setCountdown(value);
          return;
        }
        if (countdownTimerRef.current) window.clearInterval(countdownTimerRef.current);
        countdownTimerRef.current = null;
        acceptingRef.current = true;
        moveTo("listening");
        timeoutRef.current = window.setTimeout(() => {
          if (phaseRef.current === "listening") {
            moveTo("failed");
            void stop();
          }
        }, 30_000);
      }, 1_000);
    } catch (reason) {
      moveTo("failed");
      onToast(String(reason).replace(/^Error:\s*/, ""), "error");
    }
  };

  const active = phase === "starting" || phase === "countdown" || phase === "listening";
  return <div className="modal-backdrop"><section ref={dialogRef} className="calibration-dialog" role="dialog" aria-modal="true" aria-labelledby="wake-calibration-title" tabIndex={-1}>
    <button className="close-button" aria-label={t("cancel")} disabled={phase === "starting"} onClick={() => void close()}><X /></button>
    <span className={`calibration-icon ${phase}`} aria-hidden="true">{phase === "success" ? <Check /> : active ? <Mic /> : <AudioLines />}</span>
    <h2 id="wake-calibration-title">{t("wakeCalibrationTitle")}</h2>
    {phase === "intro" && <><p>{t("wakeCalibrationIntro")}</p><div className="privacy-note">{t("wakeCalibrationPrivacy")}</div></>}
    {phase === "starting" && <><LoaderCircle className="spin calibration-loader" /><strong>{t("wakeCalibrationStarting")}</strong></>}
    {phase === "countdown" && <><strong className="calibration-countdown">{countdown}</strong><p>{t("wakeCalibrationPrepare")}</p></>}
    {phase === "listening" && <>
      <strong className="calibration-prompt">Hey, AIDOO</strong>
      <p>{recognized ? t("wakeCalibrationDetectedHelp") : matches > 0 ? t("wakeCalibrationRepeat") : t("wakeCalibrationListening")}</p>
      <div className={`calibration-meter ${recognized ? "recognized" : ""}`} aria-label={t("wakeCalibrationMicrophoneActive")}><i style={{ width: `${Math.max(4, level * 100)}%` }} /></div>
      <span className={`calibration-live ${recognized ? "recognized" : hearing ? "hearing" : ""}`} role="status" aria-live="polite"><i />{recognized ? t("wakeCalibrationDetected") : hearing ? t("wakeCalibrationHearing") : heardVoice ? t("wakeCalibrationRecognizing") : matches > 0 ? t("wakeCalibrationReadyAgain") : t("wakeCalibrationWaiting")}</span>
      <div className="calibration-attempts" aria-label={t("wakeCalibrationProgress", { current: matches, total: REQUIRED_MATCHES })}>{Array.from({ length: REQUIRED_MATCHES }, (_, index) => <i key={index} className={index < matches ? "done" : ""}>{index < matches ? <Check /> : index + 1}</i>)}</div>
      <small>{t("wakeCalibrationProgress", { current: matches, total: REQUIRED_MATCHES })}</small>
    </>}
    {phase === "success" && <><strong className="calibration-result">{t("wakeCalibrationSuccess")}</strong><p>{t("wakeCalibrationSuccessHelp")}</p></>}
    {phase === "failed" && <><strong className="calibration-result">{t("wakeCalibrationFailed")}</strong><p>{t("wakeCalibrationFailedHelp")}</p></>}
    <footer>
      {phase === "intro" && <button className="primary-button" onClick={() => void start()}><Mic />{t("wakeCalibrationStart")}</button>}
      {phase === "failed" && <button className="primary-button" onClick={() => void start()}><RotateCcw />{t("retry")}</button>}
      {phase === "success" && <button className="primary-button" onClick={() => void close()}><Check />{t("finish")}</button>}
      {active && <button className="secondary-button" disabled={phase === "starting"} onClick={() => void close()}>{t("cancel")}</button>}
    </footer>
  </section></div>;
}
