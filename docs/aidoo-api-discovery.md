# AIDOO Kontrol browser discovery log

This file records only facts observed in the browser session against the test clinic. It must not contain passwords, session tokens, cookies, real patient data, full names, or screenshots with identifying data.

## Test environment

- Entry point: `https://aidoo-web.on.dev-craft.tech/clinics/demo/login`
- Environment: development test clinic
- Status: browser discovery has not started
- Code rule: no endpoint, payload, identifier, or retry behavior becomes production code before it is recorded and reproduced here.

## Session procedure

For each step, use a designated test patient and keep the browser Network panel recording. Capture the request immediately after one controlled UI action, then repeat the relevant read request to verify the result.

1. Sign in and record the authentication exchange with all secret values removed.
2. Search for one test patient using the smallest accepted query.
3. Open the patient and the active visit; record the canonical browser routes.
4. Load the complete dental status and the status/surface catalog.
5. Add one tooth-level status, then verify it with the normal read request.
6. Add one surface-level status, then verify it.
7. Replace one existing status, then verify it.
8. Apply a controlled multiple-status change, then verify every item.
9. Observe deletion only to understand the contract. Voice deletion is outside V1.

## Evidence template

Copy this block for each observed action. Replace credentials, cookies, tokens, names, birth dates, free text, and patient identifiers with stable placeholders such as `<TOKEN>` and `<TEST_PATIENT_ID>`.

```text
Action:
Observed at:
Browser route before action:
HTTP method:
Full URL and query:
Request headers (sanitised):
Request body (sanitised):
Response status:
Response body/schema (sanitised):
Follow-up read request:
Verified UI result:
Stable identifiers observed:
Ambiguities / follow-up:
```

## Observed contracts

No AIDOO API contract has been observed yet.

## Acceptance for an observed write

A write is considered understood only when the same controlled change can be reproduced, the resulting state can be read back independently, the identifier mapping is stable across a reload, and failure/timeout behavior has been observed without automatically repeating the write.
