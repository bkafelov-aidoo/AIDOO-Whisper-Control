# AIDOO Kontrol contract unknowns

These questions must be answered from the browser discovery session before the AIDOO client or voice tools are enabled. They are deliberately not assumptions in the application.

## Authentication and tenancy

- Sanitized login request and response shapes, token lifetime, expiration behavior, and logout behavior. The session endpoint and `X-Auth-Token` header name are observed.
- How the clinic UUID returned/used by the authenticated application maps to the `demo` route slug.
- Exact behavior after an expired session and whether one safe reauthentication attempt is supported.
- Production hostname and whether it differs from the development test host.

## Patients and visits

- Server-side handling below four characters, pagination, zero-result and multiple-result behavior, and the `nextAppointment` object schema. The endpoint, one-result schema, and observed four-character client minimum are recorded.
- Patient response schema and the source of the opaque patient identifier. The canonical medical-record browser route is observed.
- Business rule for when a visit becomes active or finished, and whether the dedicated active-visit endpoint is always authoritative. The read endpoint, success shape, and no-active-visit `400` are observed.
- Full contract for creating an `НЗОК` visit. Private-visit creation is observed as `POST .../patients/{patientId}/visits` with `{ doctorId }`, followed by the editable status read with `isNzok=false`.
- Whether the voice confirmation for `НЗОК` or `Частен прием` needs any additional server-side preflight before visit creation.
- Whether a separate canonical visit route exists. The status view is currently represented by `mode=status` in the patient medical-record route.

## Dental status catalog

- Authoritative source and identifier schema for the complete status catalog. The current dropdown labels are observed, but opening it did not issue a catalog request.
- Stable opaque identifiers for catalog statuses and existing status records. Teeth are observed as strings and regions as uppercase strings, but their complete allowed sets are unknown.
- Allowed tooth-level and surface-level combinations. The current UI displays single and multiple values, but writes are not yet proven.
- Localized labels and whether identifiers remain stable when labels change.
- Exact enum mapping for buccal/labial, lingual/palatal, and cervical surface labels.

## Read, write, and verification

- Whether the observed per-visit teeth-status read is the canonical complete-status read in every workflow, including when no visit is active.
- Semantics of the editable `GET .../teeth-status?visitId={visitId}&isNzok=false` response, especially why empty teeth have allocated record identifiers.
- Whether the local signer/NHIF calls are mandatory for non-NZOK status entry or are incidental to the current web flow.
- The dedicated add-status request for a tooth-level change. A controlled `Липсващ зъб` change on tooth `23` persisted in the test record, but the status editor's own request was no longer in the retained Network log. The later `PUT .../patients/{patientId}/visits/{visitId}` only finalized the visit and is not the status-write contract.
- Surface add, replacement, multiple-change, and observed delete methods and payloads. A controlled attempt to add an occlusal caries to tooth `32` did not survive read-back and therefore supplies no write-contract evidence.
- Concurrency/version fields used to reject stale edits.
- Idempotency support, if any. No idempotency behavior will be inferred.
- Validation error schema and partial-success behavior.
- Which independent read proves that a write succeeded.
- How a timeout or lost response can be resolved by read-back without repeating the write.

## Browser training exit criteria

The session is complete when sanitised fixtures cover login, search, active visit, catalog, full status read, tooth add, surface add, replacement, multiple change, validation failure, expired authentication, ambiguous write outcome, and read-back verification. Only then can the fixed-host `AidooClient`, strict semantic tools, confirmation flow, and mock contract tests be implemented safely.
