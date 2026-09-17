import { AudioLines, Link2, MessageCircle, Mic, Square } from "lucide-react";
import { translator } from "../i18n";
import type { AppLanguage, AppSettings } from "../types";
import type { LiveConversationState } from "../hooks/useLiveConversation";

export function AssistantPage({ live, language, available, dictationBusy, aidooConnected, mode }: {
  live: LiveConversationState;
  language: AppLanguage;
  available: boolean;
  dictationBusy: boolean;
  aidooConnected: boolean;
  mode: AppSettings["aidooAssistantMode"];
}) {
  const t = translator(language);
  const active = !["idle", "error"].includes(live.phase);
  const liveMode = mode === "gpt-live-1";
  const modeLabel = liveMode ? t("assistantModeLive") : t("assistantModeEconomy");
  const status = live.phase === "preparing" ? t("livePreparing")
    : live.phase === "connecting" ? t("liveConnecting")
      : live.phase === "listening" ? t("liveListening")
        : live.phase === "hearing" ? t("liveHearing")
          : live.phase === "transcribing" ? t("liveTranscribing")
        : live.phase === "speaking" ? t("liveSpeaking")
          : live.phase === "working" ? t("liveWorking")
            : live.phase === "switching" ? t("liveSwitching")
            : live.phase === "closing" ? t("liveClosing")
              : live.phase === "error" ? t("liveError")
                : t("liveReady");

  return <div className="page assistant-page">
    <header className="page-header">
      <div><span className="eyebrow">AIDOO VOICE</span><h1>{t("assistant")}</h1><p>{t("assistantTagline")}</p></div>
      <span className={`assistant-mode-pill ${active ? "active" : ""}`}><MessageCircle />{modeLabel}</span>
    </header>
    <section className={`assistant-stage ${live.phase}`} aria-live="polite">
      <div className="assistant-orb" aria-hidden="true">
        <img src="/app-icon.png" alt="" />
        {active && <div className="assistant-rings"><i /><i /><i /></div>}
      </div>
      <span className="assistant-mode-label">{modeLabel}</span>
      <h2>{status}</h2>
      <p>{active ? t("liveActiveHelp") : t("liveHelp")}</p>
      {(live.phase === "listening" || live.phase === "hearing" || live.phase === "speaking") && <div className="assistant-wave" aria-hidden="true">{Array.from({ length: 13 }, (_, index) => <i key={index} />)}</div>}
      <div className="assistant-actions">
        {active
          ? <button className="danger-button assistant-end" disabled={live.phase === "closing" || live.phase === "switching"} onClick={live.stop}><Square />{t("liveStop")}</button>
          : <button className="primary-button assistant-start" disabled={!available || dictationBusy} onClick={() => void live.start()}><AudioLines />{t("liveStart")}</button>}
      </div>
      {live.error && <div className="assistant-error" role="alert">{live.error}</div>}
      <p className="assistant-mode-cost">{liveMode ? t("liveCostGpt") : t("liveCost")}</p>
    </section>
    <section className="assistant-command-card">
      <div><Link2 /><span>{t("aidooConnection")}</span></div>
      <strong>{aidooConnected ? t("aidooConnected") : t("aidooNotConfigured")}</strong>
      <p>{t("aidooConnectionHelp")}</p>
    </section>
    <section className="assistant-command-card">
      <div><Mic /><span>{t("assistantCommandLabel")}</span></div>
      <strong>„{t("assistantCommandExample")}“</strong>
      <p>{t("assistantCommandHelp")}</p>
    </section>
  </div>;
}
