use super::{draft, treatment, types, workflow};
use crate::{acquire_operation, aidoo_keyring_entry, stop_wake_word_listener, storage, AppState};
use tauri::{AppHandle, Emitter, State};
use zeroize::Zeroizing;

#[tauri::command]
pub(crate) async fn connect_aidoo(
    clinic_slug: String,
    email: String,
    password: String,
    app: AppHandle,
    state: State<'_, AppState>,
) -> Result<types::AidooConnectionStatus, String> {
    let _operation = acquire_operation(&app, &state)?;
    stop_wake_word_listener(&state);
    let clinic_slug = clinic_slug.trim().to_string();
    let email = email.trim().to_string();
    let password = Zeroizing::new(password);
    state.aidoo.connect(&clinic_slug, &email, &password).await?;
    if let Err(error) = aidoo_keyring_entry()?.set_password(&password) {
        state.aidoo.disconnect();
        return Err(format!(
            "AIDOO паролата не можа да бъде запазена в Keychain: {error}"
        ));
    }
    let mut settings = state
        .settings
        .lock()
        .map_err(|_| "Настройките са заключени.")?
        .clone();
    settings.aidoo_clinic_slug = Some(clinic_slug.clone());
    settings.aidoo_email = Some(email.clone());
    settings.normalize();
    storage::save_settings(&settings)?;
    *state
        .settings
        .lock()
        .map_err(|_| "Настройките са заключени.")? = settings.clone();
    let _ = app.emit("settings:changed", &settings);
    Ok(types::AidooConnectionStatus {
        configured: true,
        connected: true,
        clinic_slug: Some(clinic_slug),
        email: Some(email),
    })
}

#[tauri::command]
pub(crate) fn disconnect_aidoo(
    app: AppHandle,
    state: State<'_, AppState>,
) -> Result<types::AidooConnectionStatus, String> {
    let _operation = acquire_operation(&app, &state)?;
    stop_wake_word_listener(&state);
    match aidoo_keyring_entry()?.delete_credential() {
        Ok(()) | Err(keyring::Error::NoEntry) => {}
        Err(error) => return Err(format!("AIDOO паролата не можа да бъде изтрита: {error}")),
    }
    state.aidoo.disconnect();
    let mut settings = state
        .settings
        .lock()
        .map_err(|_| "Настройките са заключени.")?
        .clone();
    settings.aidoo_clinic_slug = None;
    settings.aidoo_email = None;
    storage::save_settings(&settings)?;
    *state
        .settings
        .lock()
        .map_err(|_| "Настройките са заключени.")? = settings.clone();
    let _ = app.emit("settings:changed", &settings);
    Ok(types::AidooConnectionStatus {
        configured: false,
        connected: false,
        clinic_slug: None,
        email: None,
    })
}

#[tauri::command]
pub(crate) async fn reconnect_aidoo(
    state: State<'_, AppState>,
) -> Result<types::AidooConnectionStatus, String> {
    let settings = state
        .settings
        .lock()
        .map_err(|_| "Настройките са заключени.")?
        .clone();
    let clinic_slug = settings
        .aidoo_clinic_slug
        .ok_or_else(|| "Липсва AIDOO clinic slug.".to_string())?;
    let email = settings
        .aidoo_email
        .ok_or_else(|| "Липсва AIDOO имейл.".to_string())?;
    let password = Zeroizing::new(
        aidoo_keyring_entry()?
            .get_password()
            .map_err(|_| "Липсва AIDOO парола в Keychain.".to_string())?,
    );
    state.aidoo.connect(&clinic_slug, &email, &password).await?;
    Ok(types::AidooConnectionStatus {
        configured: true,
        connected: true,
        clinic_slug: Some(clinic_slug),
        email: Some(email),
    })
}

#[tauri::command]
pub(crate) async fn aidoo_search_patients(
    query: String,
    state: State<'_, AppState>,
) -> Result<Vec<types::PatientSearchResult>, String> {
    let session = state.aidoo.session()?;
    state
        .aidoo
        .client
        .search_patients(&session.token, &session.clinic_id, &query)
        .await
        .map_err(|error| error.message)
}

#[tauri::command]
pub(crate) async fn aidoo_status_catalog(
    state: State<'_, AppState>,
) -> Result<Vec<types::StatusCatalogEntry>, String> {
    let session = state.aidoo.session()?;
    state
        .aidoo
        .client
        .status_catalog(&session.token)
        .await
        .map_err(|error| error.message)
}

#[tauri::command]
pub(crate) async fn aidoo_diagnosis_catalog(
    state: State<'_, AppState>,
) -> Result<Vec<types::DiagnosisCatalogEntry>, String> {
    let session = state.aidoo.session()?;
    state
        .aidoo
        .client
        .diagnosis_catalog(&session.token, &session.clinic_id)
        .await
        .map_err(|error| error.message)
}

#[tauri::command]
pub(crate) async fn aidoo_procedure_catalog(
    state: State<'_, AppState>,
) -> Result<Vec<types::ProcedureCatalogEntry>, String> {
    let session = state.aidoo.session()?;
    state
        .aidoo
        .client
        .procedure_catalog(
            &session.token,
            &session.clinic_id,
            session.current_currency.as_deref(),
        )
        .await
        .map_err(|error| error.message)
}

#[tauri::command]
pub(crate) async fn aidoo_create_status_visit(
    patient_id: String,
    is_nzok: bool,
    confirmation: String,
    state: State<'_, AppState>,
) -> Result<types::StatusVisitResult, String> {
    require_spoken_confirmation(&confirmation)?;
    let session = state.aidoo.session()?;
    workflow::create_status_visit(
        &state.aidoo.client,
        &session.token,
        &session.clinic_id,
        &patient_id,
        &session.doctor_id,
        is_nzok,
    )
    .await
    .map_err(|error| error.message)
}

#[tauri::command]
pub(crate) async fn aidoo_prepare_status_draft(
    patient_id: String,
    is_nzok: bool,
    changes: Vec<types::StatusChange>,
    state: State<'_, AppState>,
) -> Result<types::PreparedStatusDraft, String> {
    let session = state.aidoo.session()?;
    let visit = state
        .aidoo
        .client
        .active_visit(&session.token, &session.clinic_id, &patient_id)
        .await
        .map_err(|error| error.message)?;
    if visit.is_finished || visit.cancelled {
        return Err("Няма активно посещение за промяна на статуса.".into());
    }
    let baseline = state
        .aidoo
        .client
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
    let catalog = state
        .aidoo
        .client
        .status_catalog(&session.token)
        .await
        .map_err(|error| error.message)?;
    let draft = draft::build_draft(patient_id, &visit, is_nzok, baseline, &catalog, &changes)?;
    let preview = types::PreparedStatusDraft {
        id: draft.id.clone(),
        spoken_summary: draft.spoken_summary.clone(),
        change_count: draft.writes.len(),
    };
    state.aidoo.store_draft(draft)?;
    Ok(preview)
}

#[tauri::command]
pub(crate) async fn aidoo_confirm_status_draft(
    draft_id: String,
    confirmation: String,
    state: State<'_, AppState>,
) -> Result<types::VerificationResult, String> {
    require_spoken_confirmation(&confirmation)?;
    let draft = state.aidoo.take_draft(&draft_id)?;
    let session = state.aidoo.session()?;
    workflow::apply_confirmed_draft(
        &state.aidoo.client,
        &session.token,
        &session.clinic_id,
        &draft,
    )
    .await
    .map_err(|error| error.message)
}

#[tauri::command]
pub(crate) fn aidoo_cancel_status_draft(state: State<'_, AppState>) {
    state.aidoo.cancel_draft();
}

#[tauri::command]
pub(crate) async fn aidoo_prepare_treatment_draft(
    patient_id: String,
    change: types::TreatmentChange,
    state: State<'_, AppState>,
) -> Result<types::PreparedTreatmentDraft, String> {
    let session = state.aidoo.session()?;
    let visit = state
        .aidoo
        .client
        .active_visit(&session.token, &session.clinic_id, &patient_id)
        .await
        .map_err(|error| error.message)?;
    if visit.is_finished || visit.cancelled {
        return Err("Няма активно посещение за запис на диагноза и процедури.".into());
    }
    let baseline = state
        .aidoo
        .client
        .visit_treatments(&session.token, &session.clinic_id, &patient_id, &visit.id)
        .await
        .map_err(|error| error.message)?;
    let diagnoses = state
        .aidoo
        .client
        .diagnosis_catalog(&session.token, &session.clinic_id)
        .await
        .map_err(|error| error.message)?;
    let procedures = state
        .aidoo
        .client
        .procedure_catalog(
            &session.token,
            &session.clinic_id,
            session.current_currency.as_deref(),
        )
        .await
        .map_err(|error| error.message)?;
    let draft = treatment::build_treatment_draft(
        patient_id,
        &visit,
        baseline,
        &diagnoses,
        &procedures,
        change,
    )?;
    let preview = types::PreparedTreatmentDraft {
        id: draft.id.clone(),
        spoken_summary: draft.spoken_summary.clone(),
        procedure_count: draft.procedures.len(),
    };
    state.aidoo.store_treatment_draft(draft)?;
    Ok(preview)
}

#[tauri::command]
pub(crate) async fn aidoo_confirm_treatment_draft(
    draft_id: String,
    confirmation: String,
    state: State<'_, AppState>,
) -> Result<types::VerificationResult, String> {
    require_spoken_confirmation(&confirmation)?;
    let draft = state.aidoo.take_treatment_draft(&draft_id)?;
    let session = state.aidoo.session()?;
    workflow::apply_confirmed_treatment_draft(
        &state.aidoo.client,
        &session.token,
        &session.clinic_id,
        &draft,
    )
    .await
    .map_err(|error| error.message)
}

#[tauri::command]
pub(crate) fn aidoo_cancel_treatment_draft(state: State<'_, AppState>) {
    state.aidoo.cancel_treatment_draft();
}

fn require_spoken_confirmation(value: &str) -> Result<(), String> {
    let normalized = value
        .to_lowercase()
        .replace(|character: char| !character.is_alphanumeric(), " ")
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ");
    if matches!(normalized.as_str(), "да" | "потвърждавам" | "потвърди") {
        Ok(())
    } else {
        Err("Действието изисква ясно гласово потвърждение.".into())
    }
}
