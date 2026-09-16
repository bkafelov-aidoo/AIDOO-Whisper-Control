use super::types::*;
use futures_util::StreamExt;
use reqwest::{Method, RequestBuilder, StatusCode};
use serde::de::DeserializeOwned;
use std::fmt;
use std::time::Duration;

const PRODUCTION_API_BASE: &str = "https://app.aidoo.bg/web";
const TEST_API_BASE: &str = "https://aidoo-platform.on.dev-craft.tech/web";
const MAX_RESPONSE_BYTES: usize = 4 * 1024 * 1024;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum AidooErrorKind {
    Authentication,
    Validation,
    NotFound,
    Http,
    Transport,
    Protocol,
}

#[derive(Debug, Clone)]
pub struct AidooError {
    pub kind: AidooErrorKind,
    pub message: String,
}

impl AidooError {
    fn authentication() -> Self {
        Self {
            kind: AidooErrorKind::Authentication,
            message: "AIDOO сесията е изтекла. Свържете профила отново.".into(),
        }
    }

    fn validation(message: impl Into<String>) -> Self {
        Self {
            kind: AidooErrorKind::Validation,
            message: message.into(),
        }
    }

    pub fn is_ambiguous_write(&self) -> bool {
        self.kind == AidooErrorKind::Transport
    }

    pub fn is_not_found(&self) -> bool {
        self.kind == AidooErrorKind::NotFound
    }
}

impl fmt::Display for AidooError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(&self.message)
    }
}

#[derive(Clone)]
pub struct AidooClient {
    http: reqwest::Client,
    api_base: String,
}

impl AidooClient {
    pub fn production() -> Result<Self, String> {
        Self::for_api_base(PRODUCTION_API_BASE)
    }

    pub fn for_api_base(api_base: &str) -> Result<Self, String> {
        if !matches!(api_base, PRODUCTION_API_BASE | TEST_API_BASE) {
            return Err("Неразпозната AIDOO среда.".into());
        }
        let http = reqwest::Client::builder()
            .https_only(true)
            .redirect(reqwest::redirect::Policy::none())
            .connect_timeout(Duration::from_secs(15))
            .timeout(Duration::from_secs(30))
            .build()
            .map_err(|error| format!("AIDOO клиентът не можа да бъде подготвен: {error}"))?;
        Ok(Self {
            http,
            api_base: api_base.into(),
        })
    }

    #[cfg(test)]
    pub fn for_test(api_base: String, timeout: Duration) -> Self {
        Self {
            http: reqwest::Client::builder()
                .redirect(reqwest::redirect::Policy::none())
                .timeout(timeout)
                .build()
                .unwrap(),
            api_base,
        }
    }

    pub async fn login(
        &self,
        clinic_slug: &str,
        email: &str,
        password: &str,
    ) -> Result<LoginResponse, AidooError> {
        validate_segment(clinic_slug, "клиника")?;
        if email.trim().is_empty() || password.is_empty() {
            return Err(AidooError::validation("Липсват AIDOO имейл или парола."));
        }
        self.send_json(
            self.http
                .post(self.url(&format!("/clinics/{clinic_slug}/sessions")))
                .json(&LoginRequest { email, password }),
        )
        .await
    }

    pub async fn search_patients(
        &self,
        session: &str,
        clinic_id: &str,
        query: &str,
    ) -> Result<Vec<PatientSearchResult>, AidooError> {
        validate_id(clinic_id, "клиника")?;
        let query = query.trim();
        if query.chars().count() < 4 {
            return Err(AidooError::validation(
                "Търсенето на пациент изисква поне четири знака.",
            ));
        }
        self.send_json(
            self.authorized(
                Method::GET,
                session,
                &format!("/clinics/{clinic_id}/patients/search"),
            )
            .query(&[("query", query)]),
        )
        .await
    }

    pub async fn active_visit(
        &self,
        session: &str,
        clinic_id: &str,
        patient_id: &str,
    ) -> Result<Visit, AidooError> {
        self.send_json(self.authorized(
            Method::GET,
            session,
            &patient_path(clinic_id, patient_id, "/visits/active")?,
        ))
        .await
    }

    pub async fn create_visit(
        &self,
        session: &str,
        clinic_id: &str,
        patient_id: &str,
        doctor_id: &str,
    ) -> Result<Visit, AidooError> {
        validate_id(doctor_id, "лекар")?;
        self.send_json(
            self.authorized(
                Method::POST,
                session,
                &patient_path(clinic_id, patient_id, "/visits")?,
            )
            .json(&CreateVisitRequest { doctor_id }),
        )
        .await
    }

    pub async fn status_catalog(
        &self,
        session: &str,
    ) -> Result<Vec<StatusCatalogEntry>, AidooError> {
        self.send_json(self.authorized(Method::GET, session, "/statuses"))
            .await
    }

    pub async fn editable_status(
        &self,
        session: &str,
        clinic_id: &str,
        patient_id: &str,
        visit_id: &str,
        is_nzok: bool,
    ) -> Result<TeethStatusResponse, AidooError> {
        validate_id(visit_id, "посещение")?;
        self.send_json(
            self.authorized(
                Method::GET,
                session,
                &patient_path(clinic_id, patient_id, "/teeth-status")?,
            )
            .query(&[("visitId", visit_id), ("isNzok", bool_text(is_nzok))]),
        )
        .await
    }

    pub async fn create_status_update(
        &self,
        session: &str,
        clinic_id: &str,
        patient_id: &str,
        visit_id: &str,
        is_nzok: bool,
    ) -> Result<TeethStatusResponse, AidooError> {
        validate_id(visit_id, "посещение")?;
        self.send_json(
            self.authorized(
                Method::POST,
                session,
                &patient_path(clinic_id, patient_id, "/teeth-status")?,
            )
            .query(&[("visitId", visit_id), ("isNzok", bool_text(is_nzok))]),
        )
        .await
    }

    pub async fn write_status(
        &self,
        session: &str,
        clinic_id: &str,
        patient_id: &str,
        visit_id: &str,
        writes: &[ToothStatusWrite],
    ) -> Result<TeethStatusResponse, AidooError> {
        if writes.is_empty() {
            return Err(AidooError::validation("Липсват статуси за запис."));
        }
        validate_id(visit_id, "посещение")?;
        self.send_json(
            self.authorized(
                Method::PUT,
                session,
                &patient_path(clinic_id, patient_id, "/teeth-status")?,
            )
            .query(&[("visitId", visit_id)])
            .json(&WriteTeethStatusRequest {
                teeth_status: writes,
            }),
        )
        .await
    }

    pub async fn visit_status(
        &self,
        session: &str,
        clinic_id: &str,
        patient_id: &str,
        visit_id: &str,
    ) -> Result<VisitTeethStatusResponse, AidooError> {
        validate_id(visit_id, "посещение")?;
        self.send_json(self.authorized(
            Method::GET,
            session,
            &patient_path(
                clinic_id,
                patient_id,
                &format!("/teeth-status/visits/{visit_id}"),
            )?,
        ))
        .await
    }

    pub async fn diagnosis_catalog(
        &self,
        session: &str,
        clinic_id: &str,
    ) -> Result<Vec<DiagnosisCatalogEntry>, AidooError> {
        validate_id(clinic_id, "клиника")?;
        self.send_json(self.authorized(
            Method::GET,
            session,
            &format!("/clinics/{clinic_id}/diagnoses"),
        ))
        .await
    }

    pub async fn procedure_catalog(
        &self,
        session: &str,
        clinic_id: &str,
        currency: Option<&str>,
    ) -> Result<Vec<ProcedureCatalogEntry>, AidooError> {
        validate_id(clinic_id, "клиника")?;
        let mut query = vec![("sortBy", "name"), ("direction", "ASC")];
        if let Some(currency) = currency.filter(|value| !value.trim().is_empty()) {
            query.push(("currency", currency));
        }
        self.send_json(
            self.authorized(
                Method::GET,
                session,
                &format!("/clinics/{clinic_id}/procedures/prices"),
            )
            .query(&query),
        )
        .await
    }

    pub async fn visit_treatments(
        &self,
        session: &str,
        clinic_id: &str,
        patient_id: &str,
        visit_id: &str,
    ) -> Result<Vec<VisitTreatment>, AidooError> {
        validate_id(visit_id, "посещение")?;
        self.send_json(self.authorized(
            Method::GET,
            session,
            &patient_path(
                clinic_id,
                patient_id,
                &format!("/visits/{visit_id}/treatments"),
            )?,
        ))
        .await
    }

    pub async fn create_treatment(
        &self,
        session: &str,
        clinic_id: &str,
        patient_id: &str,
        visit_id: &str,
        treatment: &TreatmentWrite,
    ) -> Result<VisitTreatment, AidooError> {
        validate_id(visit_id, "посещение")?;
        self.send_json(
            self.authorized(
                Method::POST,
                session,
                &patient_path(
                    clinic_id,
                    patient_id,
                    &format!("/visits/{visit_id}/treatments"),
                )?,
            )
            .json(treatment),
        )
        .await
    }

    pub async fn update_treatment(
        &self,
        session: &str,
        clinic_id: &str,
        patient_id: &str,
        visit_id: &str,
        treatment_id: &str,
        treatment: &TreatmentWrite,
    ) -> Result<VisitTreatment, AidooError> {
        validate_id(visit_id, "посещение")?;
        validate_id(treatment_id, "лечение")?;
        self.send_json(
            self.authorized(
                Method::PUT,
                session,
                &patient_path(
                    clinic_id,
                    patient_id,
                    &format!("/visits/{visit_id}/treatments/{treatment_id}"),
                )?,
            )
            .json(treatment),
        )
        .await
    }

    pub async fn add_procedure(
        &self,
        session: &str,
        clinic_id: &str,
        patient_id: &str,
        treatment_id: &str,
        procedure: &ProcedureWrite,
    ) -> Result<ProcedureCreateResponse, AidooError> {
        validate_id(treatment_id, "лечение")?;
        self.send_json(
            self.authorized(
                Method::POST,
                session,
                &patient_path(
                    clinic_id,
                    patient_id,
                    &format!("/treatments/{treatment_id}/procedures"),
                )?,
            )
            .json(procedure),
        )
        .await
    }

    pub async fn schedule_doctors(
        &self,
        session: &str,
        clinic_id: &str,
    ) -> Result<Vec<ScheduleDoctor>, AidooError> {
        validate_id(clinic_id, "клиника")?;
        self.send_json(self.authorized(
            Method::GET,
            session,
            &format!("/clinics/{clinic_id}/users"),
        ))
        .await
    }

    pub async fn search_appointments(
        &self,
        session: &str,
        clinic_id: &str,
        doctor_ids: Option<&[String]>,
        treatment_room_ids: Option<&[String]>,
        from_date: &str,
        to_date: &str,
    ) -> Result<Vec<ScheduleAppointment>, AidooError> {
        validate_id(clinic_id, "клиника")?;
        self.send_json(
            self.authorized(
                Method::POST,
                session,
                &format!("/clinics/{clinic_id}/appointments/search"),
            )
            .json(&AppointmentSearchRequest {
                doctor_ids,
                treatment_room_ids,
                from_date,
                to_date,
            }),
        )
        .await
    }

    pub async fn create_appointment(
        &self,
        session: &str,
        clinic_id: &str,
        request: &CreateAppointmentRequest<'_>,
    ) -> Result<(), AidooError> {
        validate_id(clinic_id, "клиника")?;
        self.send_without_response(
            self.authorized(
                Method::POST,
                session,
                &format!("/clinics/{clinic_id}/appointments"),
            )
            .json(request),
        )
        .await
    }

    fn authorized(&self, method: Method, session: &str, path: &str) -> RequestBuilder {
        self.http
            .request(method, self.url(path))
            .header("X-Auth-Token", session)
    }

    fn url(&self, path: &str) -> String {
        format!("{}{}", self.api_base, path)
    }

    async fn send_json<T: DeserializeOwned>(
        &self,
        request: RequestBuilder,
    ) -> Result<T, AidooError> {
        let response = request.send().await.map_err(|error| AidooError {
            kind: AidooErrorKind::Transport,
            message: format!("AIDOO не отговори: {error}"),
        })?;
        let status = response.status();
        let body = read_limited(response).await?;
        if status == StatusCode::UNAUTHORIZED {
            return Err(AidooError::authentication());
        }
        if !status.is_success() {
            return Err(AidooError {
                kind: if status == StatusCode::NOT_FOUND {
                    AidooErrorKind::NotFound
                } else {
                    AidooErrorKind::Http
                },
                message: public_http_error(status),
            });
        }
        serde_json::from_slice(&body).map_err(|_| AidooError {
            kind: AidooErrorKind::Protocol,
            message: "AIDOO върна неочакван отговор.".into(),
        })
    }

    async fn send_without_response(&self, request: RequestBuilder) -> Result<(), AidooError> {
        let response = request.send().await.map_err(|error| AidooError {
            kind: AidooErrorKind::Transport,
            message: format!("AIDOO не отговори: {error}"),
        })?;
        let status = response.status();
        let _ = read_limited(response).await?;
        if status == StatusCode::UNAUTHORIZED {
            return Err(AidooError::authentication());
        }
        if !status.is_success() {
            return Err(AidooError {
                kind: if status == StatusCode::NOT_FOUND {
                    AidooErrorKind::NotFound
                } else {
                    AidooErrorKind::Http
                },
                message: public_http_error(status),
            });
        }
        Ok(())
    }
}

async fn read_limited(response: reqwest::Response) -> Result<Vec<u8>, AidooError> {
    if response
        .content_length()
        .is_some_and(|length| length > MAX_RESPONSE_BYTES as u64)
    {
        return Err(protocol_size_error());
    }
    let mut body = Vec::new();
    let mut stream = response.bytes_stream();
    while let Some(chunk) = stream.next().await {
        let chunk = chunk.map_err(|error| AidooError {
            kind: AidooErrorKind::Transport,
            message: format!("AIDOO прекъсна отговора: {error}"),
        })?;
        if body.len().saturating_add(chunk.len()) > MAX_RESPONSE_BYTES {
            return Err(protocol_size_error());
        }
        body.extend_from_slice(&chunk);
    }
    Ok(body)
}

fn protocol_size_error() -> AidooError {
    AidooError {
        kind: AidooErrorKind::Protocol,
        message: "AIDOO върна прекалено голям отговор.".into(),
    }
}

fn public_http_error(status: StatusCode) -> String {
    match status.as_u16() {
        400 => "AIDOO отхвърли заявката. Проверете избрания пациент, посещение и статус.".into(),
        403 => "AIDOO профилът няма право за това действие.".into(),
        404 => "AIDOO не намери пациента, посещението или статуса.".into(),
        409 => "AIDOO записът е променен. Прочетете го отново и потвърдете нова чернова.".into(),
        422 => "AIDOO не прие комбинацията от статус и повърхност.".into(),
        _ if status.is_server_error() => "AIDOO временно не може да изпълни действието.".into(),
        _ => format!("AIDOO върна грешка HTTP {}.", status.as_u16()),
    }
}

fn patient_path(clinic_id: &str, patient_id: &str, suffix: &str) -> Result<String, AidooError> {
    validate_id(clinic_id, "клиника")?;
    validate_id(patient_id, "пациент")?;
    Ok(format!(
        "/clinics/{clinic_id}/patients/{patient_id}{suffix}"
    ))
}

fn validate_id(value: &str, label: &str) -> Result<(), AidooError> {
    if value.is_empty()
        || value.len() > 128
        || !value
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'_'))
    {
        return Err(AidooError::validation(format!(
            "Невалиден идентификатор за {label}."
        )));
    }
    Ok(())
}

fn validate_segment(value: &str, label: &str) -> Result<(), AidooError> {
    validate_id(value, label)
}

fn bool_text(value: bool) -> &'static str {
    if value {
        "true"
    } else {
        "false"
    }
}
