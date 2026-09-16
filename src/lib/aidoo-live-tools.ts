import { invoke } from "@tauri-apps/api/core";

interface FunctionCallItem {
  type?: string;
  call_id?: string;
  name?: string;
  arguments?: string;
}

interface ResponseUsage {
  input_tokens?: number;
  input_tokens_details?: { cached_tokens?: number; cache_write_tokens?: number };
  output_tokens?: number;
}

interface DelegatedResponse {
  id?: string;
  model?: string;
  usage?: ResponseUsage;
}

export interface LiveResponseEvent {
  type?: string;
  event?: {
    type?: string;
    item?: FunctionCallItem;
    response?: DelegatedResponse;
  };
}

export interface LiveBackendUsageEvent {
  responseId: string;
  model: string;
  usage: {
    inputTokens: number;
    cachedInputTokens: number;
    cacheWriteTokens: number;
    outputTokens: number;
  };
}

type InvokeFunction = <T>(command: string, args?: Record<string, unknown>) => Promise<T>;

const COMMANDS: Record<string, { command: string; map: (value: Record<string, unknown>) => Record<string, unknown> }> = {
  search_aidoo_patients: {
    command: "aidoo_search_patients",
    map: ({ query }) => ({ query }),
  },
  select_aidoo_patient: {
    command: "aidoo_select_patient",
    map: ({ patientId }) => ({ patientId }),
  },
  load_next_aidoo_patient: {
    command: "aidoo_next_patient",
    map: () => ({}),
  },
  begin_aidoo_status: {
    command: "aidoo_begin_status",
    map: ({ patientId }) => ({ patientId }),
  },
  start_aidoo_status_visit: {
    command: "aidoo_start_status_visit",
    map: ({ patientId, isNzok }) => ({ patientId, isNzok }),
  },
  apply_aidoo_status: {
    command: "aidoo_apply_status",
    map: ({ patientId, isNzok, change }) => ({ patientId, isNzok, change }),
  },
  finish_aidoo_status: {
    command: "aidoo_finish_status",
    map: ({ patientId }) => ({ patientId }),
  },
  add_aidoo_procedure: {
    command: "aidoo_add_procedure",
    map: ({ patientId, tooth, procedure, existingTreatmentId }) => ({ patientId, tooth, procedure, existingTreatmentId }),
  },
  write_aidoo_diagnosis: {
    command: "aidoo_write_diagnosis",
    map: ({ patientId, tooth, diagnosis, existingTreatmentId }) => ({ patientId, tooth, diagnosis, existingTreatmentId }),
  },
  write_aidoo_official_note: {
    command: "aidoo_write_official_note",
    map: ({ patientId, tooth, note, existingTreatmentId }) => ({ patientId, tooth, note, existingTreatmentId }),
  },
  find_aidoo_schedule_slot: {
    command: "aidoo_find_schedule_slot",
    map: ({ date, afterTime, durationMinutes, doctor }) => ({ date, afterTime, durationMinutes, doctor }),
  },
  book_aidoo_schedule_slot: {
    command: "aidoo_book_schedule_slot",
    map: ({ slotId, patientQuery, patientId }) => ({ slotId, patientQuery, patientId }),
  },
  get_aidoo_status_catalog: {
    command: "aidoo_status_catalog",
    map: () => ({}),
  },
  create_aidoo_status_visit: {
    command: "aidoo_create_status_visit",
    map: ({ patientId, isNzok, confirmation }) => ({ patientId, isNzok, confirmation }),
  },
  prepare_aidoo_status: {
    command: "aidoo_prepare_status_draft",
    map: ({ patientId, isNzok, changes }) => ({ patientId, isNzok, changes }),
  },
  confirm_aidoo_status: {
    command: "aidoo_confirm_status_draft",
    map: ({ draftId, confirmation }) => ({ draftId, confirmation }),
  },
  cancel_aidoo_status: {
    command: "aidoo_cancel_status_draft",
    map: () => ({}),
  },
  get_aidoo_diagnosis_catalog: {
    command: "aidoo_diagnosis_catalog",
    map: () => ({}),
  },
  get_aidoo_procedure_catalog: {
    command: "aidoo_procedure_catalog",
    map: () => ({}),
  },
  get_aidoo_active_treatments: {
    command: "aidoo_active_treatments",
    map: ({ patientId }) => ({ patientId }),
  },
  prepare_aidoo_treatment: {
    command: "aidoo_prepare_treatment_draft",
    map: ({ patientId, change }) => ({ patientId, change }),
  },
  confirm_aidoo_treatment: {
    command: "aidoo_confirm_treatment_draft",
    map: ({ draftId, confirmation }) => ({ draftId, confirmation }),
  },
  cancel_aidoo_treatment: {
    command: "aidoo_cancel_treatment_draft",
    map: () => ({}),
  },
};

export function functionCallFromLiveEvent(event: LiveResponseEvent): FunctionCallItem | null {
  if (event.type !== "response.event" || event.event?.type !== "response.output_item.done") return null;
  const item = event.event.item;
  if (item?.type !== "function_call" || !item.call_id || !item.name || typeof item.arguments !== "string") return null;
  return item;
}

export function backendUsageFromLiveEvent(event: LiveResponseEvent): LiveBackendUsageEvent | null {
  if (event.type !== "response.event" || !["response.completed", "response.incomplete", "response.failed"].includes(event.event?.type ?? "")) return null;
  const response = event.event?.response;
  const usage = response?.usage;
  const values = [
    usage?.input_tokens,
    usage?.input_tokens_details?.cached_tokens ?? 0,
    usage?.input_tokens_details?.cache_write_tokens ?? 0,
    usage?.output_tokens,
  ];
  if (!response?.id || !response.model || values.some((value) => !Number.isSafeInteger(value) || Number(value) < 0)) return null;
  const [inputTokens, cachedInputTokens, cacheWriteTokens, outputTokens] = values as number[];
  if (cachedInputTokens + cacheWriteTokens > inputTokens) return null;
  return {
    responseId: response.id,
    model: response.model,
    usage: { inputTokens, cachedInputTokens, cacheWriteTokens, outputTokens },
  };
}

export async function executeAidooLiveTool(
  item: FunctionCallItem,
  invokeFunction: InvokeFunction = invoke,
): Promise<{ callId: string; output: string }> {
  if (!item.call_id || !item.name || typeof item.arguments !== "string") {
    throw new Error("GPT-Live върна непълна заявка за AIDOO инструмент.");
  }
  const binding = COMMANDS[item.name];
  if (!binding) throw new Error("GPT-Live поиска непознат AIDOO инструмент.");
  let parsed: unknown;
  try {
    parsed = JSON.parse(item.arguments);
  } catch {
    throw new Error("GPT-Live върна невалидни аргументи за AIDOO инструмент.");
  }
  if (!parsed || typeof parsed !== "object" || Array.isArray(parsed)) {
    throw new Error("GPT-Live върна невалидни аргументи за AIDOO инструмент.");
  }
  try {
    const result = await invokeFunction<unknown>(binding.command, binding.map(parsed as Record<string, unknown>));
    return { callId: item.call_id, output: JSON.stringify({ ok: true, result: result ?? null }) };
  } catch (reason) {
    const message = String(reason).replace(/^Error:\s*/, "").slice(0, 1_000);
    return { callId: item.call_id, output: JSON.stringify({ ok: false, error: message }) };
  }
}

export function sendAidooToolOutput(channel: RTCDataChannel, callId: string, output: string) {
  channel.send(JSON.stringify({
    type: "response.item.create",
    item: { type: "function_call_output", call_id: callId, output },
  }));
  channel.send(JSON.stringify({ type: "response.create" }));
}
