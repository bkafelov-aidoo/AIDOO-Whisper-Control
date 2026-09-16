use super::client::AidooClient;
use super::types::{
    PatientSearchResult, PatientSummary, ScheduleSlot, StatusDraft, TreatmentDraft,
};
use std::sync::Mutex;
use zeroize::Zeroizing;

pub struct AidooRuntime {
    client: Mutex<AidooClient>,
    session: Mutex<Option<AidooSession>>,
    pending_draft: Mutex<Option<StatusDraft>>,
    pending_treatment_draft: Mutex<Option<TreatmentDraft>>,
    pending_schedule_slot: Mutex<Option<ScheduleSlot>>,
    patient_cursor: Mutex<PatientCursor>,
}

#[derive(Default)]
struct PatientCursor {
    recent: Vec<PatientSummary>,
    selected: Option<usize>,
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
            pending_schedule_slot: Mutex::new(None),
            patient_cursor: Mutex::new(PatientCursor::default()),
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
        self.clear_schedule_slot();
        if let Ok(mut cursor) = self.patient_cursor.lock() {
            *cursor = PatientCursor::default();
        }
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

    pub fn store_schedule_slot(&self, slot: ScheduleSlot) -> Result<(), String> {
        *self
            .pending_schedule_slot
            .lock()
            .map_err(|_| "Предложеният час е заключен.")? = Some(slot);
        Ok(())
    }

    pub fn schedule_slot(&self, slot_id: &str) -> Result<ScheduleSlot, String> {
        let pending = self
            .pending_schedule_slot
            .lock()
            .map_err(|_| "Предложеният час е заключен.")?;
        match pending.as_ref() {
            Some(slot) if slot.id == slot_id => Ok(slot.clone()),
            _ => Err("Предложеният час вече не е активен. Потърсете свободен час отново.".into()),
        }
    }

    pub fn clear_schedule_slot(&self) {
        if let Ok(mut pending) = self.pending_schedule_slot.lock() {
            *pending = None;
        }
    }

    pub fn remember_patient_search(
        &self,
        results: &[PatientSearchResult],
    ) -> Result<Option<PatientSummary>, String> {
        let mut cursor = self
            .patient_cursor
            .lock()
            .map_err(|_| "Изборът на пациент е заключен.")?;
        cursor.recent = results
            .iter()
            .map(|result| result.patient.clone())
            .collect();
        cursor.selected = (cursor.recent.len() == 1).then_some(0);
        Ok(cursor.selected.map(|index| cursor.recent[index].clone()))
    }

    pub fn select_patient(&self, patient_id: &str) -> Result<PatientSummary, String> {
        let mut cursor = self
            .patient_cursor
            .lock()
            .map_err(|_| "Изборът на пациент е заключен.")?;
        let index = cursor
            .recent
            .iter()
            .position(|patient| patient.id == patient_id)
            .ok_or_else(|| "Пациентът не е сред последните резултати от търсенето.".to_string())?;
        cursor.selected = Some(index);
        Ok(cursor.recent[index].clone())
    }

    pub fn recent_patient(&self, patient_id: &str) -> Result<PatientSummary, String> {
        let cursor = self
            .patient_cursor
            .lock()
            .map_err(|_| "Изборът на пациент е заключен.")?;
        cursor
            .recent
            .iter()
            .find(|patient| patient.id == patient_id)
            .cloned()
            .ok_or_else(|| "Пациентът не е сред последните резултати. Потърсете го отново.".into())
    }

    pub fn select_next_patient(&self) -> Result<PatientSummary, String> {
        let mut cursor = self
            .patient_cursor
            .lock()
            .map_err(|_| "Изборът на пациент е заключен.")?;
        if cursor.recent.is_empty() {
            return Err("Първо намерете пациент, за да заредите следващ резултат.".into());
        }
        let index = match cursor.selected {
            Some(index) if index + 1 < cursor.recent.len() => index + 1,
            Some(_) => return Err("Няма следващ пациент в последното търсене.".into()),
            None => 0,
        };
        cursor.selected = Some(index);
        Ok(cursor.recent[index].clone())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn result(id: &str, first_name: &str) -> PatientSearchResult {
        PatientSearchResult {
            patient: PatientSummary {
                id: id.into(),
                first_name: first_name.into(),
                middle_name: None,
                last_name: "Тестов".into(),
                mobile_phone: None,
                birthdate: None,
            },
        }
    }

    #[test]
    fn unique_search_selects_patient_and_next_walks_recent_results() {
        let runtime = AidooRuntime::new();
        let unique = runtime
            .remember_patient_search(&[result("one", "Първи")])
            .unwrap();
        assert_eq!(unique.unwrap().id, "one");

        runtime
            .remember_patient_search(&[result("one", "Първи"), result("two", "Втори")])
            .unwrap();
        assert_eq!(runtime.select_next_patient().unwrap().id, "one");
        assert_eq!(runtime.select_next_patient().unwrap().id, "two");
        assert!(runtime.select_next_patient().is_err());
    }
}
