import { useEffect, useRef, useState } from "react";
import { invoke } from "@tauri-apps/api/core";
import { open } from "@tauri-apps/plugin-dialog";
import { openUrl } from "@tauri-apps/plugin-opener";
import { isEnabled } from "@tauri-apps/plugin-autostart";
import { AudioLines, Check, CircleArrowRight, CircleHelp, ExternalLink, FileText, FolderOpen, KeyRound, Languages, LoaderCircle, Mic, Power, RefreshCw, ShieldCheck, X } from "lucide-react";
import { errorMessage, translator } from "../i18n";
import { type AppLanguage, type AppSettings, type BootstrapState, type MicrophoneProbe } from "../types";
import { type ToastHandler } from "../ui-types";
import { formatShortcut } from "../lib/presentation";
import { SettingsSection, SettingRow, Toggle, ModelPicker, StorageControls } from "../components/SettingsControls";
import { useShortcutCapture } from "../hooks/useShortcutCapture";

const SUPPORT_EMAIL_URL = "mailto:support@aidoo.bg";

export function SettingsPage({ data, language, isBusy, onSave, onRefresh, onToast, onOpenOnboarding }: {
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
  const launchAtLoginDirty = useRef(false);
  const controlsDisabled = isBusy || keyBusy || shortcutBusy || diagnosticBusy || microphoneBusy || accessibilityBusy || saveBusy;
  const shortcutButtonDisabled = isBusy || keyBusy || diagnosticBusy || microphoneBusy || accessibilityBusy || saveBusy;
  useShortcutCapture(shortcutBusy, setShortcutBusy, (binding) => setDraft((current) => ({ ...current, dictationShortcut: binding })), (message) => onToast(errorMessage(message, language), "error"));

  useEffect(() => {
    launchAtLoginDirty.current = false;
    setDraft(data.settings);
  }, [data.settings]);
  useEffect(() => {
    let disposed = false;
    const refreshLaunchAtLogin = () => {
      void isEnabled().then((enabled) => {
        if (disposed || launchAtLoginDirty.current) return;
        setDraft((current) => ({ ...current, launchAtLogin: enabled }));
      }).catch(() => undefined);
    };
    refreshLaunchAtLogin();
    window.addEventListener("focus", refreshLaunchAtLogin);
    return () => {
      disposed = true;
      window.removeEventListener("focus", refreshLaunchAtLogin);
    };
  }, [data.settings.launchAtLogin]);

  const chooseFolder = async () => {
    try {
      const selected = await open({ directory: true, multiple: false, defaultPath: draft.outputDirectory ?? data.defaultOutputDirectory });
      if (selected) setDraft({ ...draft, outputDirectory: selected });
    } catch (reason) {
      onToast(errorMessage(reason, language), "error");
    }
  };

  return <div className="page settings-page"><header className="page-header"><div><span className="eyebrow">AIDOO WHISPER LITE</span><h1>{t("settings")}</h1><p>{t("version")} {data.appVersion}</p></div><button className="secondary-button" disabled={controlsDisabled} title={isBusy ? t("finishDictationFirst") : undefined} onClick={onOpenOnboarding}><CircleArrowRight />{t("openOnboarding")}</button></header>
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
      <SettingRow title={t("launchAtLogin")}><Toggle label={t("launchAtLogin")} checked={draft.launchAtLogin} disabled={controlsDisabled} onChange={(launchAtLogin) => { launchAtLoginDirty.current = true; setDraft({ ...draft, launchAtLogin }); }} /></SettingRow>
    </SettingsSection>
    <SettingsSection icon={<ShieldCheck />} title={t("diagnostics")}>
      <p className="section-help">{t("diagnosticsHelp")}</p><div className="inline-actions"><button className="secondary-button" disabled={controlsDisabled} onClick={async () => { setDiagnosticBusy(true); try { const path = await invoke<string>("create_diagnostic_bundle"); await invoke("open_local_path", { path, reveal: true }); } catch (reason) { onToast(errorMessage(reason, language), "error"); } finally { setDiagnosticBusy(false); } }}>{diagnosticBusy ? <LoaderCircle className="spin" /> : <FileText />}{t("createDiagnostics")}</button><button className="secondary-button" disabled={controlsDisabled} onClick={async () => { try { await openUrl(SUPPORT_EMAIL_URL); } catch (reason) { onToast(errorMessage(reason, language), "error"); } }}><ExternalLink />{t("openSupport")}</button></div>
    </SettingsSection>
    <footer className="settings-footer"><button className="primary-button large" disabled={controlsDisabled} title={isBusy ? t("finishDictationFirst") : undefined} onClick={async () => { setSaveBusy(true); try { await onSave(draft); launchAtLoginDirty.current = false; } catch { /* The parent already showed the localized error. */ } finally { setSaveBusy(false); } }}>{saveBusy ? <LoaderCircle className="spin" /> : <Check />}{t("save")}</button></footer>
  </div>;
}
