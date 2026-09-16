# AIDOO Kontrol browser discovery log

This file records only facts observed in the browser session against the test clinic. It must not contain passwords, session tokens, cookies, real patient data, full names, or screenshots with identifying data.

## Test environment

- Entry point: `https://aidoo-web.on.dev-craft.tech/clinics/demo/login`
- Environment: development test clinic
- Web host: `aidoo-web.on.dev-craft.tech`
- API host: `aidoo-platform.on.dev-craft.tech`
- Status: read-only discovery in progress; no medical record has been changed
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

All identifiers below are route placeholders. No observed patient, visit, clinic, user, or session identifier is stored in this file.

### Login and session creation

- Browser entry route: `GET /clinics/{clinicSlug}/login` on the web host.
- A successful sign-in issued `POST https://aidoo-platform.on.dev-craft.tech/web/clinics/{clinicSlug}/sessions` and returned `200`.
- Authenticated API calls include an opaque token in the `X-Auth-Token` request header.
- The token value, login payload, cookies, and account data were deliberately not captured.
- Still unobserved: the sanitized login request/response schema, token lifetime, expiration behavior, and logout contract.

### Patient medical-record route

- Observed browser route:
  `/clinics/{clinicSlug}/medical-record?patientid={patientId}&tab=record&mode={treatment|status}&selectedTeeth=&triggerNzokChecksProp=true`
- Switching between `mode=treatment` and `mode=status` changed the visible table without issuing another API request once the page data was loaded.
- A clean reload of `mode=status` issued the patient, visits, and teeth-status reads described below.

### Patient search

- Search is opened from the global patient-search control without leaving the current browser route.
- The UI did not issue a request for test queries of one, two, or three characters. It issued the request after a four-character query was submitted, so the observed client-side minimum is four characters.
- Method and route:
  `GET https://aidoo-platform.on.dev-craft.tech/web/clinics/{clinicId}/patients/search?query={urlEncodedQuery}`
- Response status: `200` with `Content-Type: application/json`.
- Request body: none observed.
- Response root: JSON array.
- Each observed result has this outer shape:

```text
{
  patient: {
    id: string,
    firstName: string,
    middleName: string | null,
    lastName: string,
    birthdate: ISO-8601 date,
    email: string | null,
    mobilePhone: string | null,
    city: string | null,
    country: string | null,
    county: string | null,
    ekatte: string | null,
    street: string | null,
    streetNumber: string | null,
    neighbourhood: string | null,
    block: string | null,
    entrance: string | null,
    floor: string | null,
    apartment: string | null,
    allergies: unknown | null,
    diseases: unknown | null,
    medicalHistory: unknown | null,
    comingFrom: unknown | null,
    discount: number | null,
    identifier: string | null,
    identifierType: string | null,
    publicHealthInsured: boolean | null,
    publicHealthInsuredLastUpdated: unknown | null,
    personalInsuranceNumber: string | null,
    rzokNumber: string | null,
    healthRegion: string | null,
    pensioner: boolean,
    institutionalized: boolean,
    mentalIllness: boolean,
    gender: string,
    pending: boolean,
    balance: number | null,
    balanceCurrency: string | null
  },
  nextAppointment: object | null
}
```

- A returned result links to the canonical medical-record route using `patient.id` as the `patientid` query value.
- No pagination parameter was sent for this one-result test. Zero-result, multiple-result, pagination, and `nextAppointment` object schemas remain unobserved.
- Search results include protected health and identity data. AIDOO Control must only retain the minimum fields needed to disambiguate a patient and must never log response values.

### Patient read

- Method and route observed during a clean status-page reload:
  `GET https://aidoo-platform.on.dev-craft.tech/web/clinics/{clinicId}/patients/{patientId}`
- Response status: `200` with `Content-Type: application/json`.
- The patient response schema still needs a dedicated sanitized inspection. No patient fields or values are recorded yet.

### Visit list read

- Method and route:
  `GET https://aidoo-platform.on.dev-craft.tech/web/clinics/{clinicId}/patients/{patientId}/visits`
- Response status: `200` with `Content-Type: application/json`.
- Request body: none observed.
- Authentication header name: `X-Auth-Token`; its value is secret and is not recorded.
- Response root: JSON array.
- Fields observed on each visit include:
  - `id: string`
  - `doctor: object`
  - `price: number`
  - `priceCurrency: string`
  - `payments: array`
  - `note: string | null`
  - `timestamp: ISO-8601 string`
  - `createdStatusUpdate: boolean`
  - `isFinished: boolean`
  - `cancelled: boolean`
  - `isSentToNzis: boolean`
  - `cancelSentToNzis: boolean`
  - `nzokCompliancePassed: boolean | null`
  - `hasDentalTechnologyWorkOrder: boolean`
- The response embeds a large doctor/clinic object. AIDOO Control should model only fields required by its workflow and must not log the embedded personal or clinic data.
- The UI subsequently used one visit `id` as `{visitId}` in the teeth-status read. The rule by which that visit is selected has not yet been established.

### Active visit read

- Method and route:
  `GET https://aidoo-platform.on.dev-craft.tech/web/clinics/{clinicId}/patients/{patientId}/visits/active`
- With an active visit, the endpoint returned `200` and one visit object. The observed visit fields match the item shape from the visit-list response, including `id`, `doctor`, `timestamp`, `createdStatusUpdate`, `isFinished`, and `cancelled`.
- The observed active visit had `isFinished: false` and its `id` was used as the `visitId` for the editable teeth-status read.
- With no active visit, the same `GET` returned `400` with this sanitized shape:

```text
{
  error: "No active visit found for patient with id: <PATIENT_ID>",
  details: null
}
```

- The error embeds the patient identifier in its message. AIDOO Control must translate it to a Bulgarian user-facing message and must not log the original message.

### Complete teeth status for one visit

- Method and route:
  `GET https://aidoo-platform.on.dev-craft.tech/web/clinics/{clinicId}/patients/{patientId}/teeth-status/visits/{visitId}`
- Response status: `200` with `Content-Type: application/json`.
- Request body: none observed.
- Authentication header name: `X-Auth-Token`; its value is secret and is not recorded.
- Response root schema:

```text
{
  visitTeethStatus: Array<{
    currentToothStatus: ToothStatus,
    previousToothStatus: ToothStatus | null
  }>
}

ToothStatus = {
  id: string | null,
  tooth: string,
  statuses: string[],
  isMilkTooth: boolean,
  forObservation: boolean,
  regions: string[],
  timestamp: ISO-8601 string | null,
  note: string | null,
  generatedByProcedure: boolean
}
```

- Teeth without a current recorded status are still present. They use `id: null`, an empty `statuses` array, an empty `regions` array, and `timestamp: null`.
- `statuses` contains opaque status identifiers. One tooth may contain more than one status identifier.
- `regions` contains uppercase surface identifiers. Values observed in the read response include `OCCLUSAL` and `MESIAL`; this is evidence of values in use, not a complete allowed-value catalog.
- `previousToothStatus` either has the same `ToothStatus` shape or is `null`.
- This read was observed once after a clean reload. It must be repeated after a controlled write before it can serve as write verification evidence.

### Editable teeth status for the active visit

- Opening the existing UI flow for a new status caused the UI to resolve the active visit and then issue:
  `GET https://aidoo-platform.on.dev-craft.tech/web/clinics/{clinicId}/patients/{patientId}/teeth-status?visitId={visitId}&isNzok=false`
- Response status: `200` with `Content-Type: application/json`.
- Response root schema:

```text
{
  teethStatus: ToothStatus[]
}
```

- `ToothStatus` has the same field shape documented for the per-visit complete-status read.
- In this editable response, every tooth entry observed had a non-null `id` and timestamp, including teeth whose `statuses` and `regions` arrays were empty. This differs from the complete-status history response, where empty teeth may have `id: null` and `timestamp: null`.
- The UI also issued reads to `/nzok-checks/...` and calls to a local signer/NHIF helper on `localhost:4567` while opening this flow. These are part of the current web application's compliance flow and need separate study before AIDOO Control decides whether it should open or reproduce this UI behavior.
- Opening an already selected status dropdown did not issue a catalog API request. The labels appear to be available in the client application.
- The dropdown exposed these non-surface labels:
  `Мостоносител`, `Мостово тяло`, `Протеза`, `Свръхброен зъб`, `Липсващ зъб`, and `Шина / Адхезивен мост`.
- It exposed four surface-aware status families: `Обтурация`, `Кариес`, `Дефект на възстановяване`, and `Некариозна лезия`.
- Each surface-aware family was displayed with six localized surface choices:
  `Оклузално / Инцизално / Куспидално`, `Медиално`, `Дистално`, `Букално / Лабиално`, `Лингвално / Палатинално`, and `Цервикално`.
- Observed response enum values now include `OCCLUSAL`, `MESIAL`, and `DISTAL`. The exact enum mapping for the other localized surface choices remains unobserved.
- Existing UI rows demonstrated one status, multiple statuses, one surface, and multiple surfaces on a tooth. This establishes display capability only; allowed write combinations still require controlled write evidence.
- Closing the empty editor issued only `GET` refreshes in the captured network log. No status write was observed and no medical record value was changed.

## Acceptance for an observed write

A write is considered understood only when the same controlled change can be reproduced, the resulting state can be read back independently, the identifier mapping is stable across a reload, and failure/timeout behavior has been observed without automatically repeating the write.
