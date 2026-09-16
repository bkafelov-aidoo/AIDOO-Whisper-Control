import assert from "node:assert/strict";
import test from "node:test";
import { executeAidooLiveTool, functionCallFromLiveEvent } from "../src/lib/aidoo-live-tools.ts";

test("extracts only completed delegated function calls", () => {
  assert.equal(functionCallFromLiveEvent({ type: "response.event", event: { type: "response.output_text.delta" } }), null);
  assert.deepEqual(functionCallFromLiveEvent({
    type: "response.event",
    event: {
      type: "response.output_item.done",
      item: { type: "function_call", call_id: "call-1", name: "prepare_aidoo_status", arguments: "{}" },
    },
  }), { type: "function_call", call_id: "call-1", name: "prepare_aidoo_status", arguments: "{}" });
});

test("maps a surface status draft to the protected Tauri command", async () => {
  const calls = [];
  const result = await executeAidooLiveTool({
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

test("returns command failures to the model without retrying", async () => {
  let attempts = 0;
  const result = await executeAidooLiveTool({
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

test("maps an NZOK status visit with spoken confirmation", async () => {
  const calls = [];
  await executeAidooLiveTool({
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
  await executeAidooLiveTool({
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
  await executeAidooLiveTool({
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
  await assert.rejects(() => executeAidooLiveTool({ call_id: "x", name: "unknown", arguments: "{}" }, fakeInvoke));
  await assert.rejects(() => executeAidooLiveTool({ call_id: "x", name: "search_aidoo_patients", arguments: "not-json" }, fakeInvoke));
  assert.equal(attempts, 0);
});
