use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, BTreeSet};

pub const ALLOWED_REGIONS: &[&str] = &[
    "MESIAL",
    "DISTAL",
    "OCCLUSAL",
    "VESTIBULAR",
    "LINGUAL",
    "CERVICAL_LINGUAL",
    "CERVICAL_VESTIBULAR",
];

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct LoginResponse {
    pub session_id: String,
    pub user: LoginUser,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct LoginUser {
    pub id: String,
    pub clinic: LoginClinic,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct LoginClinic {
    pub id: String,
    #[serde(default)]
    pub current_currency: Option<String>,
}

#[derive(Debug, Clone, Serialize)]
pub struct LoginRequest<'a> {
    pub email: &'a str,
    pub password: &'a str,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct PatientSummary {
    pub id: String,
    pub first_name: String,
    #[serde(default)]
    pub middle_name: Option<String>,
    pub last_name: String,
    #[serde(default, skip_serializing)]
    pub mobile_phone: Option<String>,
    #[serde(default)]
    pub birthdate: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PatientSearchResult {
    pub patient: PatientSummary,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct Visit {
    pub id: String,
    #[serde(default)]
    pub created_status_update: bool,
    #[serde(default)]
    pub is_finished: bool,
    #[serde(default)]
    pub cancelled: bool,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct CreateVisitRequest<'a> {
    pub doctor_id: &'a str,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct StatusCatalogEntry {
    pub id: String,
    pub name: String,
    pub code: String,
    #[serde(default)]
    pub order: i64,
    #[serde(default)]
    pub diagnosis_id: Option<String>,
    #[serde(default)]
    pub can_have_regions: bool,
    #[serde(default)]
    pub regions: Vec<String>,
    #[serde(default)]
    pub incompatible_statuses: Vec<String>,
    #[serde(default, skip_serializing)]
    pub nzis_tooth_diagnosis_id: Option<String>,
}

impl StatusCatalogEntry {
    pub fn is_editable_aidoo_status(&self) -> bool {
        !self.can_have_regions || !self.regions.is_empty()
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct ToothStatus {
    #[serde(default)]
    pub id: Option<String>,
    pub tooth: String,
    #[serde(default)]
    pub statuses: Vec<String>,
    #[serde(default)]
    pub is_milk_tooth: bool,
    #[serde(default)]
    pub for_observation: bool,
    #[serde(default)]
    pub regions: Vec<String>,
    #[serde(default)]
    pub timestamp: Option<String>,
    #[serde(default)]
    pub note: Option<String>,
    #[serde(default)]
    pub generated_by_procedure: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct ToothStatusWrite {
    pub tooth: String,
    pub statuses: Vec<String>,
    pub is_milk_tooth: bool,
    pub for_observation: bool,
    pub regions: Vec<String>,
    pub note: Option<String>,
}

impl From<&ToothStatus> for ToothStatusWrite {
    fn from(value: &ToothStatus) -> Self {
        Self {
            tooth: value.tooth.clone(),
            statuses: sorted_unique(&value.statuses),
            is_milk_tooth: value.is_milk_tooth,
            for_observation: value.for_observation,
            regions: sorted_unique(&value.regions),
            note: value.note.clone(),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct TeethStatusResponse {
    #[serde(default)]
    pub teeth_status: Vec<ToothStatus>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct VisitToothStatus {
    pub current_tooth_status: ToothStatus,
    #[serde(default)]
    pub previous_tooth_status: Option<ToothStatus>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct VisitTeethStatusResponse {
    #[serde(default)]
    pub visit_teeth_status: Vec<VisitToothStatus>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct WriteTeethStatusRequest<'a> {
    pub teeth_status: &'a [ToothStatusWrite],
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub enum StatusOperation {
    Add,
    Replace,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct StatusChange {
    pub operation: StatusOperation,
    pub tooth: String,
    pub status_id: String,
    #[serde(default)]
    pub regions: Vec<String>,
    #[serde(default)]
    pub existing_status_id: Option<String>,
    #[serde(default)]
    pub is_milk_tooth: bool,
    #[serde(default)]
    pub for_observation: bool,
    #[serde(default)]
    pub note: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct StatusDraft {
    pub id: String,
    pub patient_id: String,
    pub visit_id: String,
    pub is_nzok: bool,
    pub create_status_update: bool,
    pub baseline: Vec<ToothStatus>,
    pub writes: Vec<ToothStatusWrite>,
    pub spoken_summary: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub enum VerificationOutcome {
    Verified,
    VerifiedAfterAmbiguousWrite,
    Rejected,
    Uncertain,
    StaleDraft,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct VerificationResult {
    pub outcome: VerificationOutcome,
    pub message: String,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct AidooConnectionStatus {
    pub configured: bool,
    pub connected: bool,
    pub clinic_slug: Option<String>,
    pub email: Option<String>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PreparedStatusDraft {
    pub id: String,
    pub spoken_summary: String,
    pub change_count: usize,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct StatusEntryState {
    pub ready: bool,
    pub needs_visit: bool,
    pub message: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct SpokenStatusChange {
    pub tooth: String,
    pub status: String,
    #[serde(default)]
    pub regions: Vec<String>,
    #[serde(default)]
    pub replace_status: Option<String>,
    #[serde(default)]
    pub is_milk_tooth: bool,
    #[serde(default)]
    pub for_observation: bool,
    #[serde(default)]
    pub note: Option<String>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ClinicalWriteResult {
    pub spoken_summary: String,
    pub verification: VerificationResult,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct DiagnosisCatalogEntry {
    pub id: String,
    pub name: String,
    #[serde(default, alias = "code")]
    pub key: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct ProcedureCatalogEntry {
    pub id: String,
    pub name: String,
    #[serde(default, alias = "code")]
    pub key: String,
    #[serde(default)]
    pub price: serde_json::Value,
    #[serde(default)]
    pub price_currency: Option<String>,
}

impl ProcedureCatalogEntry {
    pub fn price_text(&self) -> String {
        match &self.price {
            serde_json::Value::String(value) => value.clone(),
            serde_json::Value::Number(value) => value.to_string(),
            _ => "0".into(),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct TreatmentProcedure {
    #[serde(default)]
    pub id: Option<String>,
    pub procedure_id: String,
    #[serde(default)]
    pub price: serde_json::Value,
    #[serde(default)]
    pub discount: serde_json::Value,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct VisitTreatment {
    pub id: String,
    pub tooth: String,
    #[serde(default)]
    pub diagnosis_id: Option<String>,
    #[serde(default)]
    pub treatment_id: Option<String>,
    #[serde(default)]
    pub note: Option<String>,
    #[serde(default)]
    pub status: Option<String>,
    #[serde(default)]
    pub is_milk_tooth: bool,
    #[serde(default)]
    pub procedures: Vec<TreatmentProcedure>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct TreatmentWrite {
    pub tooth: String,
    pub diagnosis_id: Option<String>,
    pub treatment_id: Option<String>,
    pub note: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub status: Option<String>,
    pub is_milk_tooth: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct ProcedureWrite {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub id: Option<String>,
    pub procedure_id: String,
    pub price: String,
    pub discount: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct TreatmentChange {
    pub tooth: String,
    #[serde(default)]
    pub existing_treatment_id: Option<String>,
    #[serde(default)]
    pub diagnosis_id: Option<String>,
    #[serde(default)]
    pub treatment_id: Option<String>,
    #[serde(default)]
    pub note: Option<String>,
    #[serde(default)]
    pub procedure_ids: Vec<String>,
}

#[derive(Debug, Clone, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct ScheduleDoctor {
    pub id: String,
    pub first_name: String,
    pub last_name: String,
    #[serde(default)]
    pub doctor: bool,
    #[serde(default)]
    pub schedules: Vec<DoctorSchedule>,
}

#[derive(Debug, Clone, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct DoctorSchedule {
    pub treatment_room_id: String,
    pub weekday: String,
    pub start_time: String,
    pub end_time: String,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct AppointmentSearchRequest<'a> {
    pub doctor_ids: Option<&'a [String]>,
    pub treatment_room_ids: Option<&'a [String]>,
    pub from_date: &'a str,
    pub to_date: &'a str,
}

#[derive(Debug, Clone, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct ScheduleAppointment {
    pub id: String,
    pub doctor: AppointmentDoctor,
    pub treatment_room_id: String,
    pub start_time: String,
    pub end_time: String,
    #[serde(default)]
    pub patient_appointment: Option<AppointmentPatient>,
}

#[derive(Debug, Clone, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct AppointmentDoctor {
    pub id: String,
}

#[derive(Debug, Clone, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct AppointmentPatient {
    #[serde(default)]
    pub patient_id: Option<String>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct CreateAppointmentRequest<'a> {
    pub appointment_type: &'static str,
    pub doctor_id: &'a str,
    pub treatment_room_id: &'a str,
    pub start_time: &'a str,
    pub end_time: &'a str,
    pub patient_appointment: CreateAppointmentPatient<'a>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct CreateAppointmentPatient<'a> {
    pub patient_id: &'a str,
    pub first_name: &'a str,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub middle_name: Option<&'a str>,
    pub last_name: &'a str,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub mobile_phone: Option<&'a str>,
    pub status: &'static str,
    pub dental_technology_readiness: &'static str,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct ScheduleSlot {
    pub id: String,
    pub doctor_id: String,
    pub doctor_name: String,
    pub treatment_room_id: String,
    pub start_time: String,
    pub end_time: String,
    pub local_date: String,
    pub local_time: String,
    pub duration_minutes: i64,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ScheduleBookingResult {
    pub booked: bool,
    pub needs_patient_selection: bool,
    pub matches: Vec<PatientSummary>,
    pub slot: ScheduleSlot,
    pub verification: Option<VerificationResult>,
    pub spoken_summary: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct TreatmentDraft {
    pub id: String,
    pub patient_id: String,
    pub visit_id: String,
    pub existing_treatment_id: Option<String>,
    pub baseline: Vec<VisitTreatment>,
    pub treatment: TreatmentWrite,
    pub procedures: Vec<ProcedureWrite>,
    pub spoken_summary: String,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PreparedTreatmentDraft {
    pub id: String,
    pub spoken_summary: String,
    pub procedure_count: usize,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ProcedureCreateResponse {
    pub procedure: TreatmentProcedure,
    #[serde(default)]
    pub treatment_id: Option<String>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct StatusVisitResult {
    pub visit: Visit,
    pub is_nzok: bool,
    pub verification: VerificationResult,
}

pub fn sorted_unique(values: &[String]) -> Vec<String> {
    values
        .iter()
        .cloned()
        .collect::<BTreeSet<_>>()
        .into_iter()
        .collect()
}

pub fn status_map(values: &[ToothStatus]) -> BTreeMap<(String, Vec<String>), ToothStatusWrite> {
    values
        .iter()
        .map(|value| {
            let write = ToothStatusWrite::from(value);
            ((write.tooth.clone(), write.regions.clone()), write)
        })
        .collect()
}
