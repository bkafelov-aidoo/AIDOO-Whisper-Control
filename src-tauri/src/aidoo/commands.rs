use super::{
    clinic::parse_clinic_reference,
    draft,
    presentation::{self, PatientView},
    treatment, types, workflow,
};
use crate::{acquire_operation, aidoo_keyring_entry, stop_wake_word_listener, storage, AppState};
use tauri::{AppHandle, Emitter, Manager, State};
use zeroize::Zeroizing;

fn set_connection_error(state: &AppState, error: Option<String>) {
    if let Ok(mut current) = state.aidoo_connection_error.lock() {
        *current = error;
    }
}

pub(super) fn show_patient_view(
    app: &AppHandle,
    state: &AppState,
    patient_id: &str,
    view: PatientView,
) {
    let clinic_link = state.settings.lock().ok().and_then(|settings| {
        if !settings.aidoo_browser_sync_enabled {
            return None;
        }
        settings
            .aidoo_clinic_url
            .clone()
            .or_else(|| settings.aidoo_clinic_slug.clone())
    });
    if let Some(clinic_link) = clinic_link {
        presentation::present_patient(app.clone(), clinic_link, patient_id.to_string(), view);
    }
}

async fn reconnect_from_saved(state: &AppState) -> Result<types::AidooConnectionStatus, String> {
    let settings = state
        .settings
        .lock()
        .map_err(|_| "Настройките са заключени.")?
        .clone();
    let clinic_reference = settings
        .aidoo_clinic_url
        .as_deref()
        .or(settings.aidoo_clinic_slug.as_deref())
        .ok_or_else(|| "Липсва линк към AIDOO клиниката.".to_string())?;
    let clinic = parse_clinic_reference(clinic_reference)?;
    let email = settings
        .aidoo_email
        .clone()
        .ok_or_else(|| "Липсва AIDOO имейл.".to_string())?;
    let password = Zeroizing::new(
        aidoo_keyring_entry()?
            .get_password()
            .map_err(|_| "Липсва AIDOO парола в Keychain.".to_string())?,
    );
    if let Err(error) = state
        .aidoo
        .connect(&clinic.api_base, &clinic.slug, &email, &password)
        .await
    {
        set_connection_error(state, Some(error.clone()));
        return Err(error);
    }

    if settings.aidoo_clinic_url.as_deref() != Some(clinic.url.as_str())
        || settings.aidoo_clinic_slug.as_deref() != Some(clinic.slug.as_str())
    {
        let mut migrated = settings;
        migrated.aidoo_clinic_slug = Some(clinic.slug.clone());
        migrated.aidoo_clinic_url = Some(clinic.url.clone());
        storage::save_settings(&migrated)?;
        *state
            .settings
            .lock()
            .map_err(|_| "Настройките са заключени.")? = migrated;
    }

    Ok(types::AidooConnectionStatus {
        configured: true,
        connected: true,
        clinic_slug: Some(clinic.slug),
        email: Some(email),
    })
}

pub(crate) fn schedule_aidoo_auto_reconnect(app: AppHandle, refresh_existing: bool) {
    tauri::async_runtime::spawn(async move {
        tokio::time::sleep(std::time::Duration::from_millis(350)).await;
        let state = app.state::<AppState>();
        let configured = state
            .settings
            .lock()
            .map(|settings| {
                (settings.aidoo_clinic_url.is_some() || settings.aidoo_clinic_slug.is_some())
                    && settings.aidoo_email.is_some()
            })
            .unwrap_or(false);
        if !configured || (!refresh_existing && state.aidoo.connected()) {
            return;
        }
        match reconnect_from_saved(&state).await {
            Ok(_) => {
                set_connection_error(&state, None);
                storage::append_diagnostic("AIDOO Control reconnected automatically");
                let _ = app.emit("aidoo:connection-changed", true);
            }
            Err(error) => {
                state.aidoo.disconnect();
                set_connection_error(&state, Some(error.clone()));
                storage::append_diagnostic(&format!(
                    "AIDOO Control automatic reconnect failed: {error}"
                ));
                let _ = app.emit("aidoo:connection-changed", false);
            }
        }
    });
}

#[tauri::command]
pub(crate) async fn connect_aidoo(
    clinic_link: String,
    email: String,
    password: String,
    app: AppHandle,
    state: State<'_, AppState>,
) -> Result<types::AidooConnectionStatus, String> {
    let _operation = acquire_operation(&app, &state)?;
    stop_wake_word_listener(&state);
    let clinic = parse_clinic_reference(&clinic_link)?;
    let email = email.trim().to_string();
    let password = Zeroizing::new(password);
    if let Err(error) = state
        .aidoo
        .connect(&clinic.api_base, &clinic.slug, &email, &password)
        .await
    {
        set_connection_error(&state, Some(error.clone()));
        return Err(error);
    }
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
    settings.aidoo_clinic_slug = Some(clinic.slug.clone());
    settings.aidoo_clinic_url = Some(clinic.url.clone());
    settings.aidoo_email = Some(email.clone());
    settings.normalize();
    storage::save_settings(&settings)?;
    *state
        .settings
        .lock()
        .map_err(|_| "Настройките са заключени.")? = settings.clone();
    set_connection_error(&state, None);
    let _ = app.emit("settings:changed", &settings);
    let _ = app.emit("aidoo:connection-changed", true);
    Ok(types::AidooConnectionStatus {
        configured: true,
        connected: true,
        clinic_slug: Some(clinic.slug),
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
    settings.aidoo_clinic_url = None;
    settings.aidoo_email = None;
    storage::save_settings(&settings)?;
    *state
        .settings
        .lock()
        .map_err(|_| "Настройките са заключени.")? = settings.clone();
    set_connection_error(&state, None);
    let _ = app.emit("settings:changed", &settings);
    let _ = app.emit("aidoo:connection-changed", false);
    Ok(types::AidooConnectionStatus {
        configured: false,
        connected: false,
        clinic_slug: None,
        email: None,
    })
}

#[tauri::command]
pub(crate) async fn reconnect_aidoo(
    app: AppHandle,
    state: State<'_, AppState>,
) -> Result<types::AidooConnectionStatus, String> {
    match reconnect_from_saved(&state).await {
        Ok(status) => {
            set_connection_error(&state, None);
            let _ = app.emit("aidoo:connection-changed", true);
            Ok(status)
        }
        Err(error) => {
            state.aidoo.disconnect();
            set_connection_error(&state, Some(error.clone()));
            let _ = app.emit("aidoo:connection-changed", false);
            Err(error)
        }
    }
}

#[tauri::command]
pub(crate) async fn aidoo_search_patients(
    query: String,
    app: AppHandle,
    state: State<'_, AppState>,
) -> Result<Vec<types::PatientSearchResult>, String> {
    let session = state.aidoo.session()?;
    let results = state
        .aidoo
        .client()?
        .search_patients(&session.token, &session.clinic_id, &query)
        .await
        .map_err(|error| error.message)?;
    if let Some(patient) = state.aidoo.remember_patient_search(&results)? {
        show_patient_view(&app, &state, &patient.id, PatientView::Status);
    }
    Ok(results)
}

#[tauri::command]
pub(crate) async fn aidoo_status_catalog(
    state: State<'_, AppState>,
) -> Result<Vec<types::StatusCatalogEntry>, String> {
    let session = state.aidoo.session()?;
    state
        .aidoo
        .client()?
        .status_catalog(&session.token)
        .await
        .map(draft::editable_status_catalog)
        .map_err(|error| error.message)
}

#[tauri::command]
pub(crate) async fn aidoo_diagnosis_catalog(
    state: State<'_, AppState>,
) -> Result<Vec<types::DiagnosisCatalogEntry>, String> {
    let session = state.aidoo.session()?;
    state
        .aidoo
        .client()?
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
        .client()?
        .procedure_catalog(
            &session.token,
            &session.clinic_id,
            session.current_currency.as_deref(),
        )
        .await
        .map_err(|error| error.message)
}

#[tauri::command]
pub(crate) async fn aidoo_active_treatments(
    patient_id: String,
    app: AppHandle,
    state: State<'_, AppState>,
) -> Result<Vec<types::VisitTreatment>, String> {
    let session = state.aidoo.session()?;
    let client = state.aidoo.client()?;
    let visit = client
        .active_visit(&session.token, &session.clinic_id, &patient_id)
        .await
        .map_err(|error| error.message)?;
    if visit.is_finished || visit.cancelled {
        return Err("Няма активно посещение за прочит на леченията.".into());
    }
    let treatments = client
        .visit_treatments(&session.token, &session.clinic_id, &patient_id, &visit.id)
        .await
        .map_err(|error| error.message)?;
    show_patient_view(&app, &state, &patient_id, PatientView::Treatment);
    Ok(treatments)
}

#[tauri::command]
pub(crate) async fn aidoo_create_status_visit(
    patient_id: String,
    is_nzok: bool,
    confirmation: String,
    app: AppHandle,
    state: State<'_, AppState>,
) -> Result<types::StatusVisitResult, String> {
    require_spoken_confirmation(&confirmation)?;
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
pub(crate) async fn aidoo_prepare_status_draft(
    patient_id: String,
    is_nzok: bool,
    changes: Vec<types::StatusChange>,
    app: AppHandle,
    state: State<'_, AppState>,
) -> Result<types::PreparedStatusDraft, String> {
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
    let draft = draft::build_draft(patient_id, &visit, is_nzok, baseline, &catalog, &changes)?;
    let preview = types::PreparedStatusDraft {
        id: draft.id.clone(),
        spoken_summary: draft.spoken_summary.clone(),
        change_count: draft.writes.len(),
    };
    let view_patient_id = draft.patient_id.clone();
    state.aidoo.store_draft(draft)?;
    show_patient_view(&app, &state, &view_patient_id, PatientView::Status);
    Ok(preview)
}

#[tauri::command]
pub(crate) async fn aidoo_confirm_status_draft(
    draft_id: String,
    confirmation: String,
    app: AppHandle,
    state: State<'_, AppState>,
) -> Result<types::VerificationResult, String> {
    require_spoken_confirmation(&confirmation)?;
    let draft = state.aidoo.take_draft(&draft_id)?;
    let session = state.aidoo.session()?;
    let client = state.aidoo.client()?;
    let result =
        workflow::apply_confirmed_draft(&client, &session.token, &session.clinic_id, &draft).await;
    show_patient_view(&app, &state, &draft.patient_id, PatientView::Status);
    result.map_err(|error| error.message)
}

#[tauri::command]
pub(crate) fn aidoo_cancel_status_draft(state: State<'_, AppState>) {
    state.aidoo.cancel_draft();
}

#[tauri::command]
pub(crate) async fn aidoo_prepare_treatment_draft(
    patient_id: String,
    change: types::TreatmentChange,
    app: AppHandle,
    state: State<'_, AppState>,
) -> Result<types::PreparedTreatmentDraft, String> {
    let session = state.aidoo.session()?;
    let client = state.aidoo.client()?;
    let visit = client
        .active_visit(&session.token, &session.clinic_id, &patient_id)
        .await
        .map_err(|error| error.message)?;
    if visit.is_finished || visit.cancelled {
        return Err("Няма активно посещение за запис на диагноза и процедури.".into());
    }
    let baseline = client
        .visit_treatments(&session.token, &session.clinic_id, &patient_id, &visit.id)
        .await
        .map_err(|error| error.message)?;
    let diagnoses = if change.diagnosis_id.is_some() {
        client
            .diagnosis_catalog(&session.token, &session.clinic_id)
            .await
            .map_err(|error| error.message)?
    } else {
        Vec::new()
    };
    let procedures = if change.procedure_ids.is_empty() {
        Vec::new()
    } else {
        client
            .procedure_catalog(
                &session.token,
                &session.clinic_id,
                session.current_currency.as_deref(),
            )
            .await
            .map_err(|error| error.message)?
    };
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
    let view_patient_id = draft.patient_id.clone();
    state.aidoo.store_treatment_draft(draft)?;
    show_patient_view(&app, &state, &view_patient_id, PatientView::Treatment);
    Ok(preview)
}

#[tauri::command]
pub(crate) async fn aidoo_confirm_treatment_draft(
    draft_id: String,
    confirmation: String,
    app: AppHandle,
    state: State<'_, AppState>,
) -> Result<types::VerificationResult, String> {
    require_spoken_confirmation(&confirmation)?;
    let draft = state.aidoo.take_treatment_draft(&draft_id)?;
    let session = state.aidoo.session()?;
    let client = state.aidoo.client()?;
    let result = workflow::apply_confirmed_treatment_draft(
        &client,
        &session.token,
        &session.clinic_id,
        &draft,
    )
    .await;
    show_patient_view(&app, &state, &draft.patient_id, PatientView::Treatment);
    result.map_err(|error| error.message)
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
