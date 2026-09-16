# AIDOO Kontrol contract unknowns

These questions must be answered from the browser discovery session before the AIDOO client or voice tools are enabled. They are deliberately not assumptions in the application.

## Authentication and tenancy

- Login method, endpoint, request shape, cookie or token lifetime, and logout behavior.
- How the demo clinic is identified in requests and whether a clinic UUID is separate from the `demo` slug.
- Exact behavior after an expired session and whether one safe reauthentication attempt is supported.
- Production hostname and whether it differs from the development test host.

## Patients and visits

- Patient search endpoint, minimum query length, pagination, and zero/one/multiple-result schemas.
- Opaque patient identifier and canonical patient browser route.
- Definition of an active visit, its identifier, and the no-active-visit response.
- Canonical visit and dental-status browser routes.

## Dental status catalog

- Source and schema for the complete status catalog.
- Stable opaque identifiers for statuses, surfaces, teeth, and existing status records.
- Allowed tooth-level and surface-level combinations.
- Localized labels and whether identifiers remain stable when labels change.

## Read, write, and verification

- Full read endpoint and response schema for the current teeth status.
- Add, replace, multiple-change, and observed delete methods and payloads.
- Concurrency/version fields used to reject stale edits.
- Idempotency support, if any. No idempotency behavior will be inferred.
- Validation error schema and partial-success behavior.
- Which independent read proves that a write succeeded.
- How a timeout or lost response can be resolved by read-back without repeating the write.

## Browser training exit criteria

The session is complete when sanitised fixtures cover login, search, active visit, catalog, full status read, tooth add, surface add, replacement, multiple change, validation failure, expired authentication, ambiguous write outcome, and read-back verification. Only then can the fixed-host `AidooClient`, strict semantic tools, confirmation flow, and mock contract tests be implemented safely.
