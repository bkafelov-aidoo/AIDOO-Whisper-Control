import { AudioLines, CircleDollarSign, Clock3, Info, MessageCircle, ReceiptText, Server } from "lucide-react";
import { translator } from "../i18n";
import type { AppLanguage, UsageEntry, UsageLedger } from "../types";

export function UsagePage({ usage, language }: { usage: UsageLedger; language: AppLanguage }) {
  const t = translator(language);
  const aiCost = usage.liveCostNanoUsd + usage.liveBackendCostNanoUsd;
  const totalCost = aiCost + usage.transcriptionCostNanoUsd;
  const totalDuration = usage.liveDurationMillis + usage.transcriptionDurationMillis;

  return (
    <div className="page usage-page">
      <header className="page-header">
        <div>
          <span className="eyebrow">{t("usageEyebrow")}</span>
          <h1>{t("usage")}</h1>
          <p>{t("usageHelp")}</p>
        </div>
        <span className="usage-total-pill"><CircleDollarSign />{formatUsd(totalCost)}</span>
      </header>

      <section className="usage-summary" aria-label={t("usageSummary")}>
        <UsageCard
          className="live"
          icon={<MessageCircle />}
          title={t("usageAiSessions")}
          duration={formatDuration(usage.liveDurationMillis, language)}
          count={formatCount(usage.liveSessionCount, language, "session")}
          cost={formatUsd(aiCost)}
          detail={`${t("usageLiveBase")} ${formatUsd(usage.liveCostNanoUsd)} · ${t("usageBackend")} ${formatUsd(usage.liveBackendCostNanoUsd)}`}
        />
        <UsageCard
          className="transcription"
          icon={<AudioLines />}
          title={t("usageTranscriptions")}
          duration={formatDuration(usage.transcriptionDurationMillis, language)}
          count={formatCount(usage.transcriptionCount, language, "transcription")}
          cost={formatUsd(usage.transcriptionCostNanoUsd)}
        />
        <UsageCard
          className="total"
          icon={<ReceiptText />}
          title={t("usageCombined")}
          duration={formatDuration(totalDuration, language)}
          count={formatCount(usage.liveSessionCount + usage.transcriptionCount, language, "operation")}
          cost={formatUsd(totalCost)}
        />
      </section>

      <div className="usage-notes">
        <p><Info />{t("usageEstimateNote")}</p>
        <p><Info />{t("usageBackendNote")}</p>
      </div>

      <section className="section-card usage-history">
        <header>
          <div><h3>{t("usageRecent")}</h3><p>{t("usageRecentHelp")}</p></div>
          <span className="count-pill">{usage.entries.length}</span>
        </header>
        {usage.entries.length ? usage.entries.map((entry) => (
          <UsageRow key={entry.id} entry={entry} language={language} />
        )) : (
          <div className="empty-state"><Clock3 /><strong>{t("usageEmpty")}</strong><span>{t("usageEmptyHelp")}</span></div>
        )}
      </section>
    </div>
  );
}

function UsageCard({ className, icon, title, duration, count, cost, detail }: { className: string; icon: React.ReactNode; title: string; duration: string; count: string; cost: string; detail?: string }) {
  return (
    <article className={`usage-card ${className}`}>
      <div className="usage-card-icon">{icon}</div>
      <div><span>{title}</span><strong>{cost}</strong><small>{duration} · {count}{detail && <><br />{detail}</>}</small></div>
    </article>
  );
}

function UsageRow({ entry, language }: { entry: UsageEntry; language: AppLanguage }) {
  const t = translator(language);
  return (
    <article className="usage-row">
      <div className={`usage-row-icon ${entry.kind}`}>
        {entry.kind === "live" ? <MessageCircle /> : entry.kind === "liveBackend" ? <Server /> : <AudioLines />}
      </div>
      <div className="usage-row-copy">
        <strong>{entry.kind === "live" ? t("usageAiSession") : entry.kind === "liveBackend" ? t("usageBackendResponse") : t("usageTranscription")}</strong>
        <span>{formatDate(entry.createdAt, language)} · {entry.model}</span>
      </div>
      {entry.importedFromHistory && <span className="usage-imported">{t("usageImported")}</span>}
      <div className="usage-row-values">
        <strong>{formatUsd(entry.costNanoUsd)}</strong>
        <span>{entry.kind === "liveBackend" ? formatTokenUsage(entry, language) : formatDuration(entry.durationMillis, language)}</span>
      </div>
    </article>
  );
}

function formatTokenUsage(entry: UsageEntry, language: AppLanguage) {
  const formatter = new Intl.NumberFormat(language === "bg" ? "bg-BG" : "en-GB");
  const input = formatter.format(entry.inputTokens);
  const output = formatter.format(entry.outputTokens);
  return language === "bg" ? `${input} входни · ${output} изходни токена` : `${input} input · ${output} output tokens`;
}

function formatUsd(nanoUsd: number) {
  const value = nanoUsd / 1_000_000_000;
  const digits = value === 0 || value < 0.01 ? 6 : value < 1 ? 4 : 2;
  return `$${value.toFixed(digits)}`;
}

function formatDuration(milliseconds: number, language: AppLanguage) {
  const totalSeconds = Math.max(0, Math.round(milliseconds / 1000));
  const hours = Math.floor(totalSeconds / 3600);
  const minutes = Math.floor((totalSeconds % 3600) / 60);
  const seconds = totalSeconds % 60;
  const labels = language === "bg" ? { h: "ч", m: "мин", s: "сек" } : { h: "h", m: "min", s: "sec" };
  if (hours) return `${hours} ${labels.h} ${minutes} ${labels.m}`;
  if (minutes) return `${minutes} ${labels.m} ${seconds} ${labels.s}`;
  return `${seconds} ${labels.s}`;
}

function formatCount(count: number, language: AppLanguage, kind: "session" | "transcription" | "operation") {
  const labels = language === "bg"
    ? { session: count === 1 ? "сесия" : "сесии", transcription: count === 1 ? "транскрипция" : "транскрипции", operation: count === 1 ? "операция" : "операции" }
    : { session: count === 1 ? "session" : "sessions", transcription: count === 1 ? "transcription" : "transcriptions", operation: count === 1 ? "operation" : "operations" };
  return `${count} ${labels[kind]}`;
}

function formatDate(value: string, language: AppLanguage) {
  const date = new Date(value);
  if (Number.isNaN(date.getTime())) return value;
  return new Intl.DateTimeFormat(language === "bg" ? "bg-BG" : "en-GB", {
    dateStyle: "medium",
    timeStyle: "short",
  }).format(date);
}
