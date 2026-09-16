use super::client::AidooClient;
use super::types::{StatusDraft, TreatmentDraft};
use std::sync::Mutex;
use zeroize::Zeroizing;

pub struct AidooRuntime {
    client: Mutex<AidooClient>,
    session: Mutex<Option<AidooSession>>,
    pending_draft: Mutex<Option<StatusDraft>>,
    pending_treatment_draft: Mutex<Option<TreatmentDraft>>,
}

struct AidooSession {
    token: Zeroizing<String>,
    clinic_id: String,
    doctor_id: String,
    current_currency: Option<String>,
}

pub struct AidooSessionSnapshot {
    pub token: Zeroizing<String>,
    pub clinic_id: String,
    pub doctor_id: String,
    pub current_currency: Option<String>,
}

impl AidooRuntime {
    pub fn new() -> Self {
        Self {
            client: Mutex::new(
                AidooClient::production().expect("fixed AIDOO client configuration must be valid"),
            ),
            session: Mutex::new(None),
            pending_draft: Mutex::new(None),
            pending_treatment_draft: Mutex::new(None),
        }
    }

    pub async fn connect(
        &self,
        api_base: &str,
        clinic_slug: &str,
        email: &str,
        password: &str,
    ) -> Result<(), String> {
        let client = AidooClient::for_api_base(api_base)?;
        let response = client
            .login(clinic_slug, email, password)
            .await
            .map_err(|error| error.message)?;
        if response.session_id.trim().is_empty()
            || response.user.clinic.id.trim().is_empty()
            || response.user.id.trim().is_empty()
        {
            return Err("AIDOO върна непълна сесия.".into());
        }
        *self
            .client
            .lock()
            .map_err(|_| "AIDOO клиентът е заключен.")? = client;
        *self
            .session
            .lock()
            .map_err(|_| "AIDOO сесията е заключена.")? = Some(AidooSession {
            token: Zeroizing::new(response.session_id),
            clinic_id: response.user.clinic.id,
            doctor_id: response.user.id,
            current_currency: response.user.clinic.current_currency,
        });
        Ok(())
    }

    pub fn client(&self) -> Result<AidooClient, String> {
        self.client
            .lock()
            .map(|client| client.clone())
            .map_err(|_| "AIDOO клиентът е заключен.".into())
    }

    pub fn session(&self) -> Result<AidooSessionSnapshot, String> {
        let session = self
            .session
            .lock()
            .map_err(|_| "AIDOO сесията е заключена.")?;
        let session = session
            .as_ref()
            .ok_or_else(|| "Свържете AIDOO профила отново.".to_string())?;
        Ok(AidooSessionSnapshot {
            token: Zeroizing::new(session.token.to_string()),
            clinic_id: session.clinic_id.clone(),
            doctor_id: session.doctor_id.clone(),
            current_currency: session.current_currency.clone(),
        })
    }

    pub fn connected(&self) -> bool {
        self.session
            .lock()
            .map(|session| session.is_some())
            .unwrap_or(false)
    }

    pub fn disconnect(&self) {
        if let Ok(mut session) = self.session.lock() {
            *session = None;
        }
        self.cancel_draft();
        self.cancel_treatment_draft();
    }

    pub fn store_draft(&self, draft: StatusDraft) -> Result<(), String> {
        *self
            .pending_draft
            .lock()
            .map_err(|_| "AIDOO черновата е заключена.")? = Some(draft);
        Ok(())
    }

    pub fn take_draft(&self, draft_id: &str) -> Result<StatusDraft, String> {
        let mut pending = self
            .pending_draft
            .lock()
            .map_err(|_| "AIDOO черновата е заключена.")?;
        if pending.as_ref().map(|draft| draft.id.as_str()) != Some(draft_id) {
            return Err("Черновата вече не е активна.".into());
        }
        pending
            .take()
            .ok_or_else(|| "Черновата вече не е активна.".into())
    }

    pub fn cancel_draft(&self) {
        if let Ok(mut pending) = self.pending_draft.lock() {
            *pending = None;
        }
    }

    pub fn store_treatment_draft(&self, draft: TreatmentDraft) -> Result<(), String> {
        *self
            .pending_treatment_draft
            .lock()
            .map_err(|_| "AIDOO черновата за лечение е заключена.")? = Some(draft);
        Ok(())
    }

    pub fn take_treatment_draft(&self, draft_id: &str) -> Result<TreatmentDraft, String> {
        let mut pending = self
            .pending_treatment_draft
            .lock()
            .map_err(|_| "AIDOO черновата за лечение е заключена.")?;
        if pending.as_ref().map(|draft| draft.id.as_str()) != Some(draft_id) {
            return Err("Черновата за лечение вече не е активна.".into());
        }
        pending
            .take()
            .ok_or_else(|| "Черновата за лечение вече не е активна.".into())
    }

    pub fn cancel_treatment_draft(&self) {
        if let Ok(mut pending) = self.pending_treatment_draft.lock() {
            *pending = None;
        }
    }
}
