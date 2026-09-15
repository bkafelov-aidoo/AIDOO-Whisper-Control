import { progressLabel, translator } from "../i18n";
import { type AppLanguage, type BootstrapState, type ShortcutBinding } from "../types";
import { type StatusTone } from "../ui-types";

export function appIsReady(data: BootstrapState) {
  return data.settings.onboardingComplete
    && data.hasApiKey
    && data.microphones.length > 0
    && data.accessibilityGranted
    && !data.failedRecording;
}
export function appStatus(data: BootstrapState, language: AppLanguage): { tone: StatusTone; label: string } {
  const t = translator(language);
  if (data.failedRecording) return { tone: "attention", label: t("actionRequired") };
  if (data.recording.state === "starting") {
    return { tone: "busy", label: progressLabel("starting_microphone", language) };
  }
  if (data.recording.state === "recording") return { tone: "busy", label: t("recording") };
  if (data.recording.state === "transcribing") return { tone: "busy", label: t("transcribing") };
  if (data.recording.state === "done") {
    return { tone: "ready", label: progressLabel("text_ready", language) };
  }
  if (data.recording.state === "error") return { tone: "attention", label: t("errorState") };
  return appIsReady(data)
    ? { tone: "ready", label: t("ready") }
    : { tone: "attention", label: t("notReady") };
}
export function formatShortcut(binding: ShortcutBinding) {
  if (binding.code === "alt_gr") return "⌥ Right";
  const names: Record<string, string> = { meta: "⌘", shift: "⇧", alt: "⌥", control: "⌃", fn: "fn" };
  const code = binding.code.replace("key_", "").replace("num_", "").replace(/_/g, " ").toUpperCase();
  return [...binding.modifiers.map((item) => names[item] ?? item), code].join(" ");
}
export function formatDuration(seconds: number) {
  const total = Math.max(0, Math.round(seconds));
  return `${Math.floor(total / 60)}:${String(total % 60).padStart(2, "0")}`;
}
export function formatDate(value: string, language: AppLanguage) {
  const date = new Date(value);
  if (Number.isNaN(date.getTime())) return value;
  return new Intl.DateTimeFormat(language === "bg" ? "bg-BG" : "en-US", { dateStyle: "medium", timeStyle: "short" }).format(date);
}
export function modelLabel(model: string, language: AppLanguage) {
  if (model === "gpt-4o-mini-transcribe") return language === "bg" ? "Икономичен" : "Economy";
  if (model === "gpt-transcribe") return language === "bg" ? "Максимална точност" : "Maximum accuracy";
  return model;
}
