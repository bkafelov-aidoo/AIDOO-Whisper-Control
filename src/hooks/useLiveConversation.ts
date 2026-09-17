import { useCallback, useEffect, useRef, useState } from "react";
import { invoke } from "@tauri-apps/api/core";
import { listen } from "@tauri-apps/api/event";
import { executeAidooVoiceTool } from "../lib/aidoo-voice-tools";
import { LiveInactivityTimer } from "../lib/live-inactivity";
import { acquireMicrophone } from "../lib/live-microphone";
import { VoiceTurnCapture, type VoiceTurn } from "../lib/turn-audio";

export type LivePhase = "idle" | "preparing" | "connecting" | "listening" | "hearing" | "transcribing" | "speaking" | "working" | "switching" | "closing" | "error";

export interface LiveConversationState {
  phase: LivePhase;
  error: string | null;
  start: () => Promise<void>;
  stop: () => void;
}

interface VoiceToolCall {
  callId: string;
  name: string;
  arguments: string;
}

interface VoiceTurnResult {
  turnId: string | null;
  transcript: string | null;
  reply: string | null;
  action: "close" | "dictation" | null;
  toolCalls: VoiceToolCall[];
}

interface SpeechResult {
  audio: number[];
  durationSeconds: number;
  model: string;
}

const MICROPHONE_ATTEMPT_TIMEOUT_MS = 7_000;
const MICROPHONE_RETRY_DELAY_MS = 300;
const MAX_TOOL_ROUNDS = 8;

export function useLiveConversation(
  microphoneName: string | null,
  onError: (reason: unknown) => void,
  onDictationStarted?: () => void,
  onAssistantRequested?: () => void,
): LiveConversationState {
  const [phase, setPhase] = useState<LivePhase>("idle");
  const [error, setError] = useState<string | null>(null);
  const mountedRef = useRef(true);
  const operationRef = useRef(0);
  const captureRef = useRef<VoiceTurnCapture | null>(null);
  const playbackSourceRef = useRef<AudioBufferSourceNode | null>(null);
  const playbackContextRef = useRef<AudioContext | null>(null);
  const closingRef = useRef(false);
  const processingRef = useRef(false);
  const startRef = useRef<() => Promise<void>>(async () => undefined);
  const turnHandlerRef = useRef<(turn: VoiceTurn) => void>(() => undefined);
  const inactivityHandlerRef = useRef<() => void>(() => undefined);
  const inactivityTimerRef = useRef<LiveInactivityTimer | null>(null);
  if (inactivityTimerRef.current === null) {
    inactivityTimerRef.current = new LiveInactivityTimer(() => inactivityHandlerRef.current());
  }

  const updatePhase = useCallback((next: LivePhase) => {
    if (mountedRef.current) setPhase(next);
  }, []);

  useEffect(() => {
    void invoke("set_live_phase", { phase }).catch(() => undefined);
  }, [phase]);

  const stopPlayback = useCallback(() => {
    const source = playbackSourceRef.current;
    playbackSourceRef.current = null;
    if (source) {
      try { source.stop(); } catch { /* The source may already have ended. */ }
      source.disconnect();
    }
    void playbackContextRef.current?.close().catch(() => undefined);
    playbackContextRef.current = null;
  }, []);

  const releaseMedia = useCallback(() => {
    inactivityTimerRef.current?.stop();
    captureRef.current?.stop();
    captureRef.current = null;
    stopPlayback();
    processingRef.current = false;
  }, [stopPlayback]);

  const finish = useCallback((nextPhase: LivePhase = "idle") => {
    operationRef.current += 1;
    closingRef.current = true;
    releaseMedia();
    void invoke("end_live_session").catch(() => undefined);
    updatePhase(nextPhase);
  }, [releaseMedia, updatePhase]);

  const fail = useCallback((reason: unknown) => {
    const message = String(reason).replace(/^Error:\s*/, "");
    operationRef.current += 1;
    closingRef.current = true;
    releaseMedia();
    void invoke("end_live_session").catch(() => undefined);
    if (mountedRef.current) {
      setError(message);
      setPhase("error");
      onError(reason);
    }
  }, [onError, releaseMedia]);

  const playReply = useCallback(async (text: string, allowBargeIn: boolean) => {
    const speech = await invoke<SpeechResult>("synthesize_voice_reply", { text });
    if (!speech.audio.length) throw new Error("OpenAI върна празен гласов отговор.");
    const context = new AudioContext();
    playbackContextRef.current = context;
    await context.resume();
    const bytes = Uint8Array.from(speech.audio);
    const buffer = await context.decodeAudioData(bytes.buffer.slice(0));
    const source = context.createBufferSource();
    source.buffer = buffer;
    source.connect(context.destination);
    playbackSourceRef.current = source;
    if (allowBargeIn) captureRef.current?.setMode("barge-in");
    updatePhase("speaking");
    await new Promise<void>((resolve) => {
      source.onended = () => resolve();
      source.start();
    });
    if (playbackSourceRef.current === source) playbackSourceRef.current = null;
    source.disconnect();
    if (playbackContextRef.current === context) playbackContextRef.current = null;
    await context.close().catch(() => undefined);
  }, [updatePhase]);

  const playLocalGoodbye = useCallback(async () => {
    const synth = window.speechSynthesis;
    if (!synth || typeof SpeechSynthesisUtterance === "undefined") return;
    await new Promise<void>((resolve) => {
      const utterance = new SpeechSynthesisUtterance("Чао!");
      utterance.lang = "bg-BG";
      utterance.rate = 0.95;
      const timeout = window.setTimeout(resolve, 2_500);
      const complete = () => { window.clearTimeout(timeout); resolve(); };
      utterance.onend = complete;
      utterance.onerror = complete;
      try { synth.speak(utterance); } catch { complete(); }
    });
  }, []);

  const closeWithGoodbye = useCallback(async () => {
    if (closingRef.current) return;
    closingRef.current = true;
    processingRef.current = true;
    captureRef.current?.setMode("paused");
    inactivityTimerRef.current?.stop();
    updatePhase("closing");
    try {
      await playReply("Чао!", false);
    } catch {
      await playLocalGoodbye();
    }
    finish("idle");
  }, [finish, playLocalGoodbye, playReply, updatePhase]);

  const stop = useCallback(() => { void closeWithGoodbye(); }, [closeWithGoodbye]);

  const switchToDictation = useCallback(async () => {
    if (closingRef.current) return;
    closingRef.current = true;
    processingRef.current = true;
    updatePhase("switching");
    releaseMedia();
    try {
      await invoke("end_live_session");
      await invoke("start_voice_dictation");
      updatePhase("idle");
      onDictationStarted?.();
    } catch (reason) {
      fail(reason);
    }
  }, [fail, onDictationStarted, releaseMedia, updatePhase]);

  const resolveTurn = useCallback(async (initial: VoiceTurnResult, operation: number) => {
    let result = initial;
    for (let round = 0; result.toolCalls.length; round += 1) {
      if (round >= MAX_TOOL_ROUNDS) throw new Error("AIDOO командата достигна лимита за последователни действия.");
      if (operationRef.current !== operation || closingRef.current) return;
      if (!result.turnId) throw new Error("AI асистентът върна инструмент без идентификатор на репликата.");
      updatePhase("working");
      const outputs = [];
      for (const call of result.toolCalls) {
        const executed = await executeAidooVoiceTool({
          type: "function_call",
          call_id: call.callId,
          name: call.name,
          arguments: call.arguments,
        });
        outputs.push({ callId: executed.callId, output: executed.output });
      }
      result = await invoke<VoiceTurnResult>("continue_voice_turn", { turnId: result.turnId, outputs });
    }
    if (operationRef.current !== operation || closingRef.current) return;
    if (result.action === "dictation") {
      await switchToDictation();
      return;
    }
    if (result.action === "close") {
      await closeWithGoodbye();
      return;
    }
    if (result.reply) await playReply(result.reply, true);
    if (operationRef.current !== operation || closingRef.current) return;
    processingRef.current = false;
    captureRef.current?.setMode("listening");
    updatePhase("listening");
    inactivityTimerRef.current?.start();
  }, [closeWithGoodbye, playReply, switchToDictation, updatePhase]);

  const processTurn = useCallback(async (turn: VoiceTurn) => {
    if (processingRef.current || closingRef.current) return;
    processingRef.current = true;
    captureRef.current?.setMode("paused");
    inactivityTimerRef.current?.pause();
    updatePhase("transcribing");
    const operation = operationRef.current;
    try {
      const result = await invoke<VoiceTurnResult>("begin_voice_turn", { audioData: Array.from(turn.audio) });
      if (operationRef.current !== operation || closingRef.current) return;
      updatePhase("working");
      await resolveTurn(result, operation);
    } catch (reason) {
      if (operationRef.current === operation) fail(reason);
    }
  }, [fail, resolveTurn, updatePhase]);
  turnHandlerRef.current = (turn) => { void processTurn(turn); };

  inactivityHandlerRef.current = () => { void closeWithGoodbye(); };

  useEffect(() => {
    mountedRef.current = true;
    let unlistenForce: (() => void) | undefined;
    let unlistenRequest: (() => void) | undefined;
    const consumeAssistantRequest = async () => {
      try {
        if (!await invoke<boolean>("take_assistant_request")) return;
        onAssistantRequested?.();
        await startRef.current();
      } catch (reason) {
        if (mountedRef.current) onError(reason);
      }
    };
    const onFocus = () => { void consumeAssistantRequest(); };
    void listen<string>("live:force-close", () => finish("idle")).then((dispose) => { unlistenForce = dispose; });
    void listen("assistant:requested", () => { void consumeAssistantRequest(); }).then((dispose) => { unlistenRequest = dispose; });
    window.addEventListener("focus", onFocus);
    void consumeAssistantRequest();
    return () => {
      mountedRef.current = false;
      unlistenForce?.();
      unlistenRequest?.();
      window.removeEventListener("focus", onFocus);
      operationRef.current += 1;
      releaseMedia();
      void invoke("end_live_session").catch(() => undefined);
    };
  }, [finish, onAssistantRequested, onError, releaseMedia]);

  const start = useCallback(async () => {
    if (!["idle", "error"].includes(phase)) return;
    const operation = operationRef.current + 1;
    operationRef.current = operation;
    const stillCurrent = () => operationRef.current === operation && mountedRef.current;
    setError(null);
    closingRef.current = false;
    processingRef.current = false;
    updatePhase("preparing");
    try {
      await invoke("prepare_live_session");
      if (!stillCurrent()) return;
      updatePhase("connecting");
      const audioConstraints: MediaTrackConstraints = {
        echoCancellation: true,
        noiseSuppression: true,
        autoGainControl: true,
      };
      if (microphoneName) {
        const devices = await navigator.mediaDevices.enumerateDevices();
        const selected = devices.find((device) => device.kind === "audioinput" && device.label === microphoneName)
          ?? devices.find((device) => device.kind === "audioinput" && device.label.includes(microphoneName));
        if (selected?.deviceId) audioConstraints.deviceId = { exact: selected.deviceId };
      }
      const microphone = await acquireMicrophone(
        (constraints) => navigator.mediaDevices.getUserMedia(constraints),
        { audio: audioConstraints },
        { attemptTimeoutMs: MICROPHONE_ATTEMPT_TIMEOUT_MS, retryDelayMs: MICROPHONE_RETRY_DELAY_MS },
      );
      if (!stillCurrent()) {
        microphone.getTracks().forEach((track) => track.stop());
        return;
      }
      const capture = new VoiceTurnCapture({
        onSpeechStart: () => {
          if (playbackSourceRef.current) {
            operationRef.current += 1;
            processingRef.current = false;
            stopPlayback();
          }
          updatePhase("hearing");
        },
        onActivity: () => inactivityTimerRef.current?.touch(),
        onTurn: (turn) => turnHandlerRef.current(turn),
      });
      captureRef.current = capture;
      await capture.start(microphone);
      if (!stillCurrent()) {
        capture.stop();
        return;
      }
      updatePhase("listening");
      inactivityTimerRef.current?.start();
    } catch (reason) {
      if (stillCurrent()) fail(reason);
    }
  }, [fail, microphoneName, phase, stopPlayback, updatePhase]);
  startRef.current = start;

  return { phase, error, start, stop };
}
