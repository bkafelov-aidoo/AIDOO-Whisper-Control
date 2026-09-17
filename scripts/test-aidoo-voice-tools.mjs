import assert from "node:assert/strict";
import test from "node:test";
import { executeAidooVoiceTool } from "../src/lib/aidoo-voice-tools.ts";

test("maps a surface status draft to the protected Tauri command", async () => {
  const calls = [];
  const result = await executeAidooVoiceTool({
    type: "function_call",
    call_id: "call-2",
    name: "prepare_aidoo_status",
    arguments: JSON.stringify({
      patientId: "patient-test",
      isNzok: false,
      changes: [{
        operation: "add",
        tooth: "32",
        statusId: "caries-test",
        regions: ["OCCLUSAL"],
        existingStatusId: null,
        isMilkTooth: false,
        forObservation: false,
        note: null,
      }],
    }),
  }, async (command, args) => {
    calls.push({ command, args });
    return { id: "draft-test", spokenSummary: "Потвърдете", changeCount: 1 };
  });
  assert.equal(calls.length, 1);
  assert.equal(calls[0].command, "aidoo_prepare_status_draft");
  assert.deepEqual(calls[0].args.changes[0].regions, ["OCCLUSAL"]);
  assert.deepEqual(JSON.parse(result.output), {
    ok: true,
    result: { id: "draft-test", spokenSummary: "Потвърдете", changeCount: 1 },
  });
});

test("maps the direct clinical protocol without confirmation round trips", async () => {
  const calls = [];
  const invoke = async (command, args) => {
    calls.push({ command, args });
    return { verification: { outcome: "verified" } };
  };
  await executeAidooVoiceTool({
    type: "function_call",
    call_id: "call-direct-status",
    name: "apply_aidoo_status",
    arguments: JSON.stringify({
      patientId: "patient-test",
      isNzok: false,
      change: {
        tooth: "16",
        status: "Кариес",
        regions: ["OCCLUSAL"],
        replaceStatus: null,
        isMilkTooth: false,
        forObservation: false,
        note: null,
      },
    }),
  }, invoke);
  await executeAidooVoiceTool({
    type: "function_call",
    call_id: "call-finish-status",
    name: "finish_aidoo_status",
    arguments: JSON.stringify({ patientId: "patient-test" }),
  }, invoke);
  await executeAidooVoiceTool({
    type: "function_call",
    call_id: "call-direct-note",
    name: "write_aidoo_official_note",
    arguments: JSON.stringify({
      patientId: "patient-test",
      tooth: "*",
      note: "Пациентът е информиран.",
      existingTreatmentId: null,
    }),
  }, invoke);

  assert.deepEqual(calls, [
    {
      command: "aidoo_apply_status",
      args: {
        patientId: "patient-test",
        isNzok: false,
        change: {
          tooth: "16",
          status: "Кариес",
          regions: ["OCCLUSAL"],
          replaceStatus: null,
          isMilkTooth: false,
          forObservation: false,
          note: null,
        },
      },
    },
    { command: "aidoo_finish_status", args: { patientId: "patient-test" } },
    {
      command: "aidoo_write_official_note",
      args: {
        patientId: "patient-test",
        tooth: "*",
        note: "Пациентът е информиран.",
        existingTreatmentId: null,
      },
    },
  ]);
});

test("returns command failures to the model without retrying", async () => {
  let attempts = 0;
  const result = await executeAidooVoiceTool({
    type: "function_call",
    call_id: "call-3",
    name: "confirm_aidoo_status",
    arguments: JSON.stringify({ draftId: "draft-test", confirmation: "Да" }),
  }, async () => {
    attempts += 1;
    throw new Error("Записът не можа да бъде потвърден.");
  });
  assert.equal(attempts, 1);
  assert.deepEqual(JSON.parse(result.output), { ok: false, error: "Записът не можа да бъде потвърден." });
});

test("maps visible schedule discovery and the explicit booking command", async () => {
  const calls = [];
  const invoke = async (command, args) => {
    calls.push({ command, args });
    return { ok: true };
  };
  await executeAidooVoiceTool({
    type: "function_call",
    call_id: "call-find-slot",
    name: "find_aidoo_schedule_slot",
    arguments: JSON.stringify({
      date: null,
      afterTime: "12:00",
      durationMinutes: 30,
      doctor: null,
    }),
  }, invoke);
  await executeAidooVoiceTool({
    type: "function_call",
    call_id: "call-book-slot",
    name: "book_aidoo_schedule_slot",
    arguments: JSON.stringify({
      slotId: "slot-test",
      patientQuery: "Тест Пациент",
      patientId: null,
    }),
  }, invoke);
  assert.deepEqual(calls, [
    {
      command: "aidoo_find_schedule_slot",
      args: { date: null, afterTime: "12:00", durationMinutes: 30, doctor: null },
    },
    {
      command: "aidoo_book_schedule_slot",
      args: { slotId: "slot-test", patientQuery: "Тест Пациент", patientId: null },
    },
  ]);
});

test("maps an NZOK status visit with spoken confirmation", async () => {
  const calls = [];
  await executeAidooVoiceTool({
    type: "function_call",
    call_id: "call-visit",
    name: "create_aidoo_status_visit",
    arguments: JSON.stringify({ patientId: "patient-test", isNzok: true, confirmation: "Да" }),
  }, async (command, args) => {
    calls.push({ command, args });
    return { isNzok: true };
  });
  assert.deepEqual(calls, [{
    command: "aidoo_create_status_visit",
    args: { patientId: "patient-test", isNzok: true, confirmation: "Да" },
  }]);
});

test("maps diagnosis, procedures and dictated official note as one draft", async () => {
  const calls = [];
  const change = {
    tooth: "26",
    existingTreatmentId: "treatment-1",
    diagnosisId: "diagnosis-1",
    treatmentId: null,
    note: "Пациентът е информиран за възможностите.",
    procedureIds: ["procedure-1"],
  };
  await executeAidooVoiceTool({
    type: "function_call",
    call_id: "call-treatment",
    name: "prepare_aidoo_treatment",
    arguments: JSON.stringify({ patientId: "patient-test", change }),
  }, async (command, args) => {
    calls.push({ command, args });
    return { id: "draft-1" };
  });
  assert.deepEqual(calls, [{
    command: "aidoo_prepare_treatment_draft",
    args: { patientId: "patient-test", change },
  }]);
});

test("reads active treatment rows before selecting one", async () => {
  const calls = [];
  await executeAidooVoiceTool({
    type: "function_call",
    call_id: "call-active-treatments",
    name: "get_aidoo_active_treatments",
    arguments: JSON.stringify({ patientId: "patient-test" }),
  }, async (command, args) => {
    calls.push({ command, args });
    return [{ id: "row-a", tooth: "26" }, { id: "row-b", tooth: "26" }];
  });
  assert.deepEqual(calls, [{
    command: "aidoo_active_treatments",
    args: { patientId: "patient-test" },
  }]);
});

test("rejects unknown tools and malformed arguments before invoking native code", async () => {
  let attempts = 0;
  const fakeInvoke = async () => { attempts += 1; };
  await assert.rejects(() => executeAidooVoiceTool({ call_id: "x", name: "unknown", arguments: "{}" }, fakeInvoke));
  await assert.rejects(() => executeAidooVoiceTool({ call_id: "x", name: "search_aidoo_patients", arguments: "not-json" }, fakeInvoke));
  assert.equal(attempts, 0);
});
