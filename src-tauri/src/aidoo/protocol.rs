use super::{
    commands::show_patient_view, draft, presentation::PatientView, treatment, types, workflow,
};
use crate::AppState;
use tauri::{AppHandle, State};

#[tauri::command]
pub(crate) fn aidoo_select_patient(
    patient_id: String,
    app: AppHandle,
    state: State<'_, AppState>,
) -> Result<types::PatientSummary, String> {
    let patient = state.aidoo.select_patient(&patient_id)?;
    show_patient_view(&app, &state, &patient.id, PatientView::Status);
    Ok(patient)
}

#[tauri::command]
pub(crate) fn aidoo_next_patient(
    app: AppHandle,
    state: State<'_, AppState>,
) -> Result<types::PatientSummary, String> {
    let patient = state.aidoo.select_next_patient()?;
    show_patient_view(&app, &state, &patient.id, PatientView::Status);
    Ok(patient)
}

#[tauri::command]
pub(crate) async fn aidoo_begin_status(
    patient_id: String,
    app: AppHandle,
    state: State<'_, AppState>,
) -> Result<types::StatusEntryState, String> {
    let session = state.aidoo.session()?;
    let client = state.aidoo.client()?;
    let result = client
        .active_visit(&session.token, &session.clinic_id, &patient_id)
        .await;
    show_patient_view(&app, &state, &patient_id, PatientView::Status);
    match result {
        Ok(visit) if !visit.is_finished && !visit.cancelled => Ok(types::StatusEntryState {
            ready: true,
            needs_visit: false,
            message: "Статусът е готов за попълване.".into(),
        }),
        Err(error) if error.is_not_found() => Ok(types::StatusEntryState {
            ready: false,
            needs_visit: true,
            message: "Няма активно посещение. Попитайте само дали приемът е частен или по НЗОК."
                .into(),
        }),
        Ok(_) => Err("Няма активно посещение за попълване на статус.".into()),
        Err(error) => Err(error.message),
    }
}

#[tauri::command]
pub(crate) async fn aidoo_start_status_visit(
    patient_id: String,
    is_nzok: bool,
    app: AppHandle,
    state: State<'_, AppState>,
) -> Result<types::StatusVisitResult, String> {
    let session = state.aidoo.session()?;
    let client = state.aidoo.client()?;
    let result = workflow::create_status_visit(
        &client,
        &session.token,
        &session.clinic_id,
        &patient_id,
        &session.doctor_id,
        is_nzok,
    )
    .await;
    show_patient_view(&app, &state, &patient_id, PatientView::Status);
    result.map_err(|error| error.message)
}

#[tauri::command]
pub(crate) async fn aidoo_apply_status(
    patient_id: String,
    is_nzok: bool,
    change: types::SpokenStatusChange,
    app: AppHandle,
    state: State<'_, AppState>,
) -> Result<types::ClinicalWriteResult, String> {
    let session = state.aidoo.session()?;
    let client = state.aidoo.client()?;
    let visit = client
        .active_visit(&session.token, &session.clinic_id, &patient_id)
        .await
        .map_err(|error| error.message)?;
    if visit.is_finished || visit.cancelled {
        return Err("Няма активно посещение за промяна на статуса.".into());
    }
    let baseline = client
        .editable_status(
            &session.token,
            &session.clinic_id,
            &patient_id,
            &visit.id,
            is_nzok,
        )
        .await
        .map_err(|error| error.message)?
        .teeth_status;
    let catalog = client
        .status_catalog(&session.token)
        .await
        .map(draft::editable_status_catalog)
        .map_err(|error| error.message)?;
    let status = resolve_status(&catalog, &change.status)?;
    let existing_status_id = change
        .replace_status
        .as_deref()
        .map(|query| resolve_status(&catalog, query).map(|entry| entry.id.clone()))
        .transpose()?;
    let operation = if existing_status_id.is_some() {
        types::StatusOperation::Replace
    } else {
        types::StatusOperation::Add
    };
    let draft = draft::build_draft(
        patient_id.clone(),
        &visit,
        is_nzok,
        baseline,
        &catalog,
        &[types::StatusChange {
            operation,
            tooth: change.tooth,
            status_id: status.id.clone(),
            regions: change.regions,
            existing_status_id,
            is_milk_tooth: change.is_milk_tooth,
            for_observation: change.for_observation,
            note: change.note,
        }],
    )?;
    let result =
        workflow::apply_confirmed_draft(&client, &session.token, &session.clinic_id, &draft).await;
    show_patient_view(&app, &state, &patient_id, PatientView::Status);
    Ok(clinical_result(
        &draft.spoken_summary,
        result.map_err(|error| error.message)?,
    ))
}

#[tauri::command]
pub(crate) fn aidoo_finish_status(
    patient_id: String,
    app: AppHandle,
    state: State<'_, AppState>,
) -> String {
    show_patient_view(&app, &state, &patient_id, PatientView::Treatment);
    "Статусът е записан. Отварям Лечение.".into()
}

#[tauri::command]
pub(crate) async fn aidoo_add_procedure(
    patient_id: String,
    tooth: String,
    procedure: String,
    existing_treatment_id: Option<String>,
    app: AppHandle,
    state: State<'_, AppState>,
) -> Result<types::ClinicalWriteResult, String> {
    let session = state.aidoo.session()?;
    let client = state.aidoo.client()?;
    let visit = active_treatment_visit(&client, &session, &patient_id).await?;
    let baseline = client
        .visit_treatments(&session.token, &session.clinic_id, &patient_id, &visit.id)
        .await
        .map_err(|error| error.message)?;
    let procedures = client
        .procedure_catalog(
            &session.token,
            &session.clinic_id,
            session.current_currency.as_deref(),
        )
        .await
        .map_err(|error| error.message)?;
    let procedure_id = resolve_procedure(&procedures, &procedure)?.id.clone();
    let row = resolve_treatment_row(&baseline, &tooth, existing_treatment_id.as_deref())?;
    apply_treatment_change(
        &app,
        &state,
        &session,
        &client,
        patient_id,
        visit,
        baseline,
        Vec::new(),
        procedures,
        types::TreatmentChange {
            tooth,
            existing_treatment_id: row,
            diagnosis_id: None,
            treatment_id: None,
            note: None,
            procedure_ids: vec![procedure_id],
        },
    )
    .await
}

#[tauri::command]
pub(crate) async fn aidoo_write_diagnosis(
    patient_id: String,
    tooth: String,
    diagnosis: String,
    existing_treatment_id: Option<String>,
    app: AppHandle,
    state: State<'_, AppState>,
) -> Result<types::ClinicalWriteResult, String> {
    let session = state.aidoo.session()?;
    let client = state.aidoo.client()?;
    let visit = active_treatment_visit(&client, &session, &patient_id).await?;
    let baseline = client
        .visit_treatments(&session.token, &session.clinic_id, &patient_id, &visit.id)
        .await
        .map_err(|error| error.message)?;
    let diagnoses = client
        .diagnosis_catalog(&session.token, &session.clinic_id)
        .await
        .map_err(|error| error.message)?;
    let diagnosis_id = resolve_diagnosis(&diagnoses, &diagnosis)?.id.clone();
    let row = resolve_treatment_row(&baseline, &tooth, existing_treatment_id.as_deref())?;
    apply_treatment_change(
        &app,
        &state,
        &session,
        &client,
        patient_id,
        visit,
        baseline,
        diagnoses,
        Vec::new(),
        types::TreatmentChange {
            tooth,
            existing_treatment_id: row,
            diagnosis_id: Some(diagnosis_id),
            treatment_id: None,
            note: None,
            procedure_ids: Vec::new(),
        },
    )
    .await
}

#[tauri::command]
pub(crate) async fn aidoo_write_official_note(
    patient_id: String,
    tooth: String,
    note: String,
    existing_treatment_id: Option<String>,
    app: AppHandle,
    state: State<'_, AppState>,
) -> Result<types::ClinicalWriteResult, String> {
    let session = state.aidoo.session()?;
    let client = state.aidoo.client()?;
    let visit = active_treatment_visit(&client, &session, &patient_id).await?;
    let baseline = client
        .visit_treatments(&session.token, &session.clinic_id, &patient_id, &visit.id)
        .await
        .map_err(|error| error.message)?;
    let row = resolve_treatment_row(&baseline, &tooth, existing_treatment_id.as_deref())?;
    apply_treatment_change(
        &app,
        &state,
        &session,
        &client,
        patient_id,
        visit,
        baseline,
        Vec::new(),
        Vec::new(),
        types::TreatmentChange {
            tooth,
            existing_treatment_id: row,
            diagnosis_id: None,
            treatment_id: None,
            note: Some(note),
            procedure_ids: Vec::new(),
        },
    )
    .await
}

async fn active_treatment_visit(
    client: &super::client::AidooClient,
    session: &super::runtime::AidooSessionSnapshot,
    patient_id: &str,
) -> Result<types::Visit, String> {
    let visit = client
        .active_visit(&session.token, &session.clinic_id, patient_id)
        .await
        .map_err(|error| error.message)?;
    if visit.is_finished || visit.cancelled {
        return Err("Няма активно посещение за запис в Лечение.".into());
    }
    Ok(visit)
}

#[allow(clippy::too_many_arguments)]
async fn apply_treatment_change(
    app: &AppHandle,
    state: &AppState,
    session: &super::runtime::AidooSessionSnapshot,
    client: &super::client::AidooClient,
    patient_id: String,
    visit: types::Visit,
    baseline: Vec<types::VisitTreatment>,
    diagnoses: Vec<types::DiagnosisCatalogEntry>,
    procedures: Vec<types::ProcedureCatalogEntry>,
    change: types::TreatmentChange,
) -> Result<types::ClinicalWriteResult, String> {
    let draft = treatment::build_treatment_draft(
        patient_id.clone(),
        &visit,
        baseline,
        &diagnoses,
        &procedures,
        change,
    )?;
    let result = workflow::apply_confirmed_treatment_draft(
        client,
        &session.token,
        &session.clinic_id,
        &draft,
    )
    .await;
    show_patient_view(app, state, &patient_id, PatientView::Treatment);
    Ok(clinical_result(
        &draft.spoken_summary,
        result.map_err(|error| error.message)?,
    ))
}

fn clinical_result(
    draft_summary: &str,
    verification: types::VerificationResult,
) -> types::ClinicalWriteResult {
    let spoken_summary = if matches!(
        verification.outcome,
        types::VerificationOutcome::Verified
            | types::VerificationOutcome::VerifiedAfterAmbiguousWrite
    ) {
        completed_summary(draft_summary)
    } else {
        verification.message.clone()
    };
    types::ClinicalWriteResult {
        spoken_summary,
        verification,
    }
}

fn completed_summary(value: &str) -> String {
    let value = value
        .strip_prefix("Ще запиша ")
        .unwrap_or(value)
        .strip_suffix(". Да го запиша ли?")
        .unwrap_or(value);
    format!("Записано: {value}.")
}

fn normalized(value: &str) -> String {
    value
        .to_lowercase()
        .replace(|character: char| !character.is_alphanumeric(), " ")
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ")
}

fn unique_catalog_match<'a, T>(
    entries: &'a [T],
    query: &str,
    name: impl Fn(&T) -> &str,
    key: impl Fn(&T) -> &str,
    label: &str,
) -> Result<&'a T, String> {
    let query = normalized(query);
    if query.is_empty() {
        return Err(format!("Липсва {label} за търсене."));
    }
    let exact = entries
        .iter()
        .filter(|entry| normalized(name(entry)) == query || normalized(key(entry)) == query)
        .collect::<Vec<_>>();
    if exact.len() == 1 {
        return Ok(exact[0]);
    }
    let partial = entries
        .iter()
        .filter(|entry| {
            normalized(name(entry)).contains(&query) || normalized(key(entry)).contains(&query)
        })
        .collect::<Vec<_>>();
    if partial.len() == 1 {
        return Ok(partial[0]);
    }
    Err(if exact.len() > 1 || partial.len() > 1 {
        format!("{label} е двусмислена. Уточнете пълното име.")
    } else {
        format!("{label} не е намерена в актуалния AIDOO каталог.")
    })
}

fn resolve_status<'a>(
    entries: &'a [types::StatusCatalogEntry],
    query: &str,
) -> Result<&'a types::StatusCatalogEntry, String> {
    unique_catalog_match(
        entries,
        query,
        |entry| &entry.name,
        |entry| &entry.code,
        "Статусът",
    )
}

fn resolve_procedure<'a>(
    entries: &'a [types::ProcedureCatalogEntry],
    query: &str,
) -> Result<&'a types::ProcedureCatalogEntry, String> {
    unique_catalog_match(
        entries,
        query,
        |entry| &entry.name,
        |entry| &entry.key,
        "Процедурата",
    )
}

fn resolve_diagnosis<'a>(
    entries: &'a [types::DiagnosisCatalogEntry],
    query: &str,
) -> Result<&'a types::DiagnosisCatalogEntry, String> {
    unique_catalog_match(
        entries,
        query,
        |entry| &entry.name,
        |entry| &entry.key,
        "Диагнозата",
    )
}

fn resolve_treatment_row(
    entries: &[types::VisitTreatment],
    tooth: &str,
    explicit_id: Option<&str>,
) -> Result<Option<String>, String> {
    if let Some(id) = explicit_id {
        let row = entries
            .iter()
            .find(|entry| entry.id == id)
            .ok_or_else(|| "Избраният ред за лечение вече не съществува.".to_string())?;
        if row.tooth != tooth {
            return Err("Избраният ред за лечение е за друг зъб.".into());
        }
        return Ok(Some(id.to_string()));
    }
    let matches = entries
        .iter()
        .filter(|entry| entry.tooth == tooth)
        .collect::<Vec<_>>();
    match matches.as_slice() {
        [] => Ok(None),
        [entry] => Ok(Some(entry.id.clone())),
        _ => Err(format!(
            "Има {} реда в Лечение за {}. Уточнете кой ред да се промени.",
            matches.len(),
            if tooth == "*" {
                "звездичката".into()
            } else {
                format!("зъб {tooth}")
            }
        )),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn catalog_matching_accepts_full_name_code_and_one_unique_partial() {
        let entries = vec![
            types::ProcedureCatalogEntry {
                id: "one".into(),
                name: "Професионално почистване".into(),
                key: "PROC-1".into(),
                price: serde_json::json!(10),
                price_currency: Some("BGN".into()),
            },
            types::ProcedureCatalogEntry {
                id: "two".into(),
                name: "Обтурация".into(),
                key: "PROC-2".into(),
                price: serde_json::json!(20),
                price_currency: Some("BGN".into()),
            },
        ];
        assert_eq!(resolve_procedure(&entries, "proc 1").unwrap().id, "one");
        assert_eq!(resolve_procedure(&entries, "обтурац").unwrap().id, "two");
        assert!(resolve_procedure(&entries, "липсваща").is_err());
    }

    #[test]
    fn treatment_row_is_automatic_only_when_unambiguous() {
        let row = |id: &str| types::VisitTreatment {
            id: id.into(),
            tooth: "16".into(),
            diagnosis_id: None,
            treatment_id: None,
            note: None,
            status: None,
            is_milk_tooth: false,
            procedures: Vec::new(),
        };
        assert_eq!(resolve_treatment_row(&[], "16", None).unwrap(), None);
        assert_eq!(
            resolve_treatment_row(&[row("one")], "16", None).unwrap(),
            Some("one".into())
        );
        assert!(resolve_treatment_row(&[row("one"), row("two")], "16", None).is_err());
    }

    #[test]
    fn spoken_result_claims_saved_only_after_verified_read_back() {
        let verified = clinical_result(
            "Ще запиша кариес на зъб 16. Да го запиша ли?",
            types::VerificationResult {
                outcome: types::VerificationOutcome::Verified,
                message: "verified".into(),
            },
        );
        assert_eq!(verified.spoken_summary, "Записано: кариес на зъб 16.");

        let rejected = clinical_result(
            "Ще запиша кариес на зъб 16. Да го запиша ли?",
            types::VerificationResult {
                outcome: types::VerificationOutcome::Rejected,
                message: "Записът не е потвърден.".into(),
            },
        );
        assert_eq!(rejected.spoken_summary, "Записът не е потвърден.");
    }
}
