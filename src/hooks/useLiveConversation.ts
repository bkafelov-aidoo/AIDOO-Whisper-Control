import { useCallback, useEffect, useRef, useState, type MutableRefObject } from "react";
import { invoke } from "@tauri-apps/api/core";
import { listen } from "@tauri-apps/api/event";
import {
  ASSISTANT_CLOSE_GRACE_MS,
  AssistantVoiceCommandDetector,
  detectAssistantVoiceCommandFromLiveEvent,
} from "../lib/assistant-command";
import { backendUsageFromLiveEvent, executeAidooLiveTool, functionCallFromLiveEvent, sendAidooToolOutput } from "../lib/aidoo-live-tools";
import {
  ASSISTANT_GOODBYE_SILENCE_MS,
  ASSISTANT_GOODBYE_START_TIMEOUT_MS,
  LiveInactivityTimer,
  assistantGoodbyeInstruction,
  assistantGoodbyePrompt,
} from "../lib/live-inactivity";
import { acquireMicrophone } from "../lib/live-microphone";

export type LivePhase = "idle" | "preparing" | "connecting" | "listening" | "speaking" | "working" | "switching" | "closing" | "error";

export interface LiveConversationState {
  phase: LivePhase;
  error: string | null;
  start: () => Promise<void>;
  stop: () => void;
}

interface LiveSessionAnswer {
  sessionId: string;
  sdp: string;
}

interface LiveEvent {
  type?: string;
  client_event_id?: string;
  delta?: string;
  error?: { message?: string };
  event?: {
    type?: string;
    item?: { type?: string; call_id?: string; name?: string; arguments?: string };
    response?: {
      id?: string;
      model?: string;
      usage?: {
        input_tokens?: number;
        input_tokens_details?: { cached_tokens?: number; cache_write_tokens?: number };
        output_tokens?: number;
      };
    };
  };
}

const MICROPHONE_ATTEMPT_TIMEOUT_MS = 7_000;
const MICROPHONE_RETRY_DELAY_MS = 300;
const ICE_GATHERING_TIMEOUT_MS = 10_000;
const LIVE_CREATE_TIMEOUT_MS = 50_000;
const SESSION_START_TIMEOUT_MS = 20_000;

export function useLiveConversation(
  microphoneName: string | null,
  onError: (reason: unknown) => void,
  onDictationStarted?: () => void,
  onAssistantRequested?: () => void,
): LiveConversationState {
  const [phase, setPhase] = useState<LivePhase>("idle");
  const [error, setError] = useState<string | null>(null);
  const peerRef = useRef<RTCPeerConnection | null>(null);
  const channelRef = useRef<RTCDataChannel | null>(null);
  const microphoneRef = useRef<MediaStream | null>(null);
  const audioContextRef = useRef<AudioContext | null>(null);
  const monitorFrameRef = useRef<number | null>(null);
  const timeoutRef = useRef<number | null>(null);
  const goodbyeTimerRef = useRef<number | null>(null);
  const goodbyeInstructionIdRef = useRef<string | null>(null);
  const readyRef = useRef(false);
  const closingRef = useRef(false);
  const switchingRef = useRef(false);
  const toolBusyRef = useRef(false);
  const mountedRef = useRef(true);
  const operationRef = useRef(0);
  const commandDetectorRef = useRef(new AssistantVoiceCommandDetector());
  const startRef = useRef<() => Promise<void>>(async () => undefined);
  const stopRef = useRef<() => void>(() => undefined);
  const inactivityHandlerRef = useRef<() => void>(() => undefined);
  const inactivityTimerRef = useRef<LiveInactivityTimer | null>(null);
  const handledToolCallsRef = useRef(new Set<string>());
  if (inactivityTimerRef.current === null) {
    inactivityTimerRef.current = new LiveInactivityTimer(() => inactivityHandlerRef.current());
  }

  const updatePhase = useCallback((next: LivePhase) => {
    if (mountedRef.current) setPhase(next);
  }, []);

  useEffect(() => {
    void invoke("set_live_phase", { phase }).catch(() => undefined);
  }, [phase]);

  const clearTimer = useCallback(() => {
    if (timeoutRef.current !== null) window.clearTimeout(timeoutRef.current);
    timeoutRef.current = null;
  }, []);

  const clearGoodbyeTimer = useCallback(() => {
    if (goodbyeTimerRef.current !== null) window.clearTimeout(goodbyeTimerRef.current);
    goodbyeTimerRef.current = null;
  }, []);

  const releaseBrowserMedia = useCallback(() => {
    clearTimer();
    clearGoodbyeTimer();
    inactivityTimerRef.current?.stop();
    readyRef.current = false;
    if (monitorFrameRef.current !== null) cancelAnimationFrame(monitorFrameRef.current);
    monitorFrameRef.current = null;
    microphoneRef.current?.getTracks().forEach((track) => track.stop());
    microphoneRef.current = null;
    channelRef.current?.close();
    channelRef.current = null;
    peerRef.current?.close();
    peerRef.current = null;
    void audioContextRef.current?.close().catch(() => undefined);
    audioContextRef.current = null;
    commandDetectorRef.current.reset();
    handledToolCallsRef.current.clear();
    toolBusyRef.current = false;
    goodbyeInstructionIdRef.current = null;
  }, [clearGoodbyeTimer, clearTimer]);

  const finish = useCallback((nextPhase: LivePhase = "idle") => {
    operationRef.current += 1;
    closingRef.current = true;
    switchingRef.current = false;
    releaseBrowserMedia();
    void invoke("end_live_session").catch(() => undefined);
    updatePhase(nextPhase);
  }, [releaseBrowserMedia, updatePhase]);

  const fail = useCallback((reason: unknown) => {
    const message = String(reason).replace(/^Error:\s*/, "");
    operationRef.current += 1;
    closingRef.current = true;
    switchingRef.current = false;
    releaseBrowserMedia();
    void invoke("end_live_session").catch(() => undefined);
    if (mountedRef.current) {
      setError(message);
      setPhase("error");
      onError(reason);
    }
  }, [onError, releaseBrowserMedia]);

  const switchToDictation = useCallback(async () => {
    if (switchingRef.current) return;
    switchingRef.current = true;
    closingRef.current = true;
    operationRef.current += 1;
    updatePhase("switching");
    releaseBrowserMedia();
    try {
      await invoke("end_live_session");
      await invoke("start_voice_dictation");
      switchingRef.current = false;
      updatePhase("idle");
      onDictationStarted?.();
    } catch (reason) {
      fail(reason);
    }
  }, [fail, onDictationStarted, releaseBrowserMedia, updatePhase]);

  const stop = useCallback(() => {
    if (closingRef.current) return;
    operationRef.current += 1;
    closingRef.current = true;
    updatePhase("closing");
    microphoneRef.current?.getTracks().forEach((track) => track.stop());
    microphoneRef.current = null;
    const channel = channelRef.current;
    if (readyRef.current && channel?.readyState === "open") {
      try {
        channel.send(JSON.stringify({ type: "session.close" }));
      } catch {
        finish("idle");
        return;
      }
      clearTimer();
      timeoutRef.current = window.setTimeout(() => finish("idle"), ASSISTANT_CLOSE_GRACE_MS);
    } else {
      finish("idle");
    }
  }, [clearTimer, finish, updatePhase]);
  stopRef.current = stop;

  const finishAfterLocalGoodbye = useCallback(() => {
    clearGoodbyeTimer();
    const synth = window.speechSynthesis;
    if (!synth || typeof SpeechSynthesisUtterance === "undefined") {
      finish("idle");
      return;
    }
    const utterance = new SpeechSynthesisUtterance("Чао!");
    utterance.lang = "bg-BG";
    utterance.rate = 0.95;
    let completed = false;
    const complete = () => {
      if (completed) return;
      completed = true;
      clearGoodbyeTimer();
      finish("idle");
    };
    utterance.onend = complete;
    utterance.onerror = complete;
    goodbyeTimerRef.current = window.setTimeout(complete, 2_500);
    try {
      synth.speak(utterance);
    } catch {
      complete();
    }
  }, [clearGoodbyeTimer, finish]);

  const beginInactiveClose = useCallback(() => {
    if (closingRef.current) return;
    const channel = channelRef.current;
    if (toolBusyRef.current) {
      inactivityTimerRef.current?.start();
      return;
    }
    if (!readyRef.current || channel?.readyState !== "open") {
      finish("idle");
      return;
    }
    closingRef.current = true;
    updatePhase("closing");
    const eventId = `aidoo_idle_goodbye_${Date.now()}`;
    goodbyeInstructionIdRef.current = eventId;
    try {
      channel.send(JSON.stringify(assistantGoodbyeInstruction(eventId)));
    } catch {
      finishAfterLocalGoodbye();
      return;
    }
    clearGoodbyeTimer();
    goodbyeTimerRef.current = window.setTimeout(finishAfterLocalGoodbye, ASSISTANT_GOODBYE_START_TIMEOUT_MS);
  }, [clearGoodbyeTimer, finish, finishAfterLocalGoodbye, updatePhase]);
  inactivityHandlerRef.current = beginInactiveClose;

  const registerRemoteSpeech = useCallback(() => {
    if (!closingRef.current || !goodbyeInstructionIdRef.current) {
      inactivityTimerRef.current?.touch();
      return;
    }
    clearGoodbyeTimer();
    goodbyeTimerRef.current = window.setTimeout(() => finish("idle"), ASSISTANT_GOODBYE_SILENCE_MS);
  }, [clearGoodbyeTimer, finish]);

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
      releaseBrowserMedia();
      void invoke("end_live_session").catch(() => undefined);
    };
  }, [finish, onAssistantRequested, onError, releaseBrowserMedia]);

  const start = useCallback(async () => {
    if (!["idle", "error"].includes(phase)) return;
    const operation = operationRef.current + 1;
    operationRef.current = operation;
    const stillCurrent = () => operationRef.current === operation && mountedRef.current;
    setError(null);
    updatePhase("preparing");
    closingRef.current = false;
    switchingRef.current = false;
    commandDetectorRef.current.reset();
    try {
      await invoke("prepare_live_session");
      if (!stillCurrent()) return;
      await delay(180);
      if (!stillCurrent()) return;

      const peer = new RTCPeerConnection();
      peerRef.current = peer;
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
        {
          attemptTimeoutMs: MICROPHONE_ATTEMPT_TIMEOUT_MS,
          retryDelayMs: MICROPHONE_RETRY_DELAY_MS,
        },
      );
      if (!stillCurrent()) {
        microphone.getTracks().forEach((track) => track.stop());
        return;
      }
      microphoneRef.current = microphone;
      for (const track of microphone.getAudioTracks()) peer.addTrack(track, microphone);
      updatePhase("connecting");

      peer.addEventListener("track", ({ track }) => monitorRemoteAudio(track, audioContextRef, monitorFrameRef, readyRef, closingRef, toolBusyRef, mountedRef, updatePhase, registerRemoteSpeech));

      const channel = peer.createDataChannel("oai-events");
      channelRef.current = channel;
      channel.addEventListener("message", ({ data }) => {
        if (typeof data !== "string") return;
        let event: LiveEvent;
        try { event = JSON.parse(data) as LiveEvent; } catch { return; }
        if (event.type === "session.started") {
          clearTimer();
          readyRef.current = true;
          updatePhase("listening");
          inactivityTimerRef.current?.start();
        } else if (event.type === "session.input_transcript.delta" && event.delta) {
          inactivityTimerRef.current?.touch();
          const command = detectAssistantVoiceCommandFromLiveEvent(event, commandDetectorRef.current);
          if (command === "start-dictation") void switchToDictation();
          if (command === "end-session") stopRef.current();
        } else if (event.type === "session.instructions.appended" && event.client_event_id === goodbyeInstructionIdRef.current) {
          const promptId = `${event.client_event_id}_prompt`;
          try {
            channel.send(JSON.stringify(assistantGoodbyePrompt(promptId)));
          } catch {
            finishAfterLocalGoodbye();
          }
        } else if (event.type === "response.event") {
          inactivityTimerRef.current?.touch();
          const backendUsage = backendUsageFromLiveEvent(event);
          if (backendUsage) {
            void invoke("record_live_backend_usage", {
              responseId: backendUsage.responseId,
              model: backendUsage.model,
              usage: backendUsage.usage,
            }).catch(() => undefined);
          }
          const call = functionCallFromLiveEvent(event);
          if (!call?.call_id || handledToolCallsRef.current.has(call.call_id)) return;
          handledToolCallsRef.current.add(call.call_id);
          toolBusyRef.current = true;
          inactivityTimerRef.current?.pause();
          updatePhase("working");
          void executeAidooLiveTool(call).then(({ callId, output }) => {
            if (channel.readyState !== "open" || closingRef.current) return;
            sendAidooToolOutput(channel, callId, output);
            toolBusyRef.current = false;
            updatePhase("listening");
            inactivityTimerRef.current?.start();
          }).catch((reason) => {
            toolBusyRef.current = false;
            fail(reason);
          });
        } else if (event.type === "session.closed") {
          finish("idle");
        } else if (event.type === "error" || event.type === "session.failed") {
          if (goodbyeInstructionIdRef.current) finishAfterLocalGoodbye();
          else fail(event.error?.message ?? "GPT-Live прекъсна разговора.");
        }
      });
      channel.addEventListener("close", () => {
        if (readyRef.current && !closingRef.current) fail("GPT-Live връзката беше прекъсната.");
      });
      peer.addEventListener("connectionstatechange", () => {
        if (peer.connectionState === "failed" && !closingRef.current) fail("WebRTC връзката с GPT-Live беше прекъсната.");
      });

      const offer = await peer.createOffer();
      await peer.setLocalDescription(offer);
      await waitForIceGathering(peer);
      if (!stillCurrent()) return;
      const sdp = peer.localDescription?.sdp;
      if (!sdp) throw new Error("WebRTC не създаде заявка за разговор.");
      const answer = await withTimeout(
        invoke<LiveSessionAnswer>("create_live_session", { sdp }),
        LIVE_CREATE_TIMEOUT_MS,
        "OpenAI не отговори навреме.",
      );
      if (!stillCurrent()) return;
      await peer.setRemoteDescription({ type: "answer", sdp: answer.sdp });
      if (!readyRef.current) {
        timeoutRef.current = window.setTimeout(() => fail("GPT-Live не потвърди старта на разговора."), SESSION_START_TIMEOUT_MS);
      }
    } catch (reason) {
      if (stillCurrent()) fail(reason);
    }
  }, [clearTimer, fail, finish, finishAfterLocalGoodbye, microphoneName, phase, registerRemoteSpeech, switchToDictation, updatePhase]);
  startRef.current = start;

  return { phase, error, start, stop };
}

function monitorRemoteAudio(
  track: MediaStreamTrack,
  audioContextRef: MutableRefObject<AudioContext | null>,
  monitorFrameRef: MutableRefObject<number | null>,
  readyRef: MutableRefObject<boolean>,
  closingRef: MutableRefObject<boolean>,
  toolBusyRef: MutableRefObject<boolean>,
  mountedRef: MutableRefObject<boolean>,
  updatePhase: (phase: LivePhase) => void,
  onRemoteSpeech: () => void,
) {
  try {
    const context = new AudioContext();
    audioContextRef.current = context;
    const source = context.createMediaStreamSource(new MediaStream([track]));
    const analyser = context.createAnalyser();
    analyser.fftSize = 256;
    source.connect(analyser);
    analyser.connect(context.destination);
    void context.resume();
    const samples = new Uint8Array(analyser.fftSize);
    let lastSpeechAt = 0;
    const monitor = () => {
      analyser.getByteTimeDomainData(samples);
      let energy = 0;
      for (const sample of samples) {
        const centered = (sample - 128) / 128;
        energy += centered * centered;
      }
      if (Math.sqrt(energy / samples.length) > 0.025) {
        lastSpeechAt = performance.now();
        onRemoteSpeech();
      }
      if (readyRef.current && !closingRef.current && !toolBusyRef.current && mountedRef.current) {
        updatePhase(performance.now() - lastSpeechAt < 280 ? "speaking" : "listening");
      }
      monitorFrameRef.current = requestAnimationFrame(monitor);
    };
    monitor();
  } catch {
    // The conversation stays usable even if the visual audio meter is unavailable.
  }
}

function waitForIceGathering(peer: RTCPeerConnection) {
  if (peer.iceGatheringState === "complete") return Promise.resolve();
  return new Promise<void>((resolve, reject) => {
    const timeout = window.setTimeout(() => {
      peer.removeEventListener("icegatheringstatechange", onState);
      reject(new Error("WebRTC не успя да подготви мрежовата връзка."));
    }, ICE_GATHERING_TIMEOUT_MS);
    function onState() {
      if (peer.iceGatheringState !== "complete") return;
      window.clearTimeout(timeout);
      peer.removeEventListener("icegatheringstatechange", onState);
      resolve();
    }
    peer.addEventListener("icegatheringstatechange", onState);
    onState();
  });
}

function withTimeout<T>(promise: Promise<T>, timeoutMs: number, message: string): Promise<T> {
  return new Promise<T>((resolve, reject) => {
    const timeout = window.setTimeout(() => reject(new Error(message)), timeoutMs);
    promise.then(
      (value) => { window.clearTimeout(timeout); resolve(value); },
      (reason) => { window.clearTimeout(timeout); reject(reason); },
    );
  });
}

function delay(milliseconds: number) {
  return new Promise<void>((resolve) => window.setTimeout(resolve, milliseconds));
}
