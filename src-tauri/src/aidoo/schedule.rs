use super::{client::AidooClient, presentation, runtime::AidooSessionSnapshot, types};
use crate::AppState;
use chrono::{
    DateTime, Datelike, Duration, Local, NaiveDate, NaiveDateTime, NaiveTime, TimeZone, Timelike,
    Utc, Weekday,
};
use tauri::{AppHandle, State};

const DEFAULT_DURATION_MINUTES: i64 = 30;
const SEARCH_HORIZON_DAYS: i64 = 30;
const SLOT_STEP_MINUTES: i64 = 15;

#[tauri::command]
pub(crate) async fn aidoo_find_schedule_slot(
    date: Option<String>,
    after_time: String,
    duration_minutes: Option<i64>,
    doctor: Option<String>,
    app: AppHandle,
    state: State<'_, AppState>,
) -> Result<types::ScheduleSlot, String> {
    state.aidoo.clear_schedule_slot();
    let session = state.aidoo.session()?;
    let client = state.aidoo.client()?;
    let now = Local::now();
    let (from_date, to_date) = search_dates(date.as_deref(), now.date_naive())?;
    let after_time = parse_time(&after_time)?;
    let duration_minutes = validate_duration(duration_minutes.unwrap_or(DEFAULT_DURATION_MINUTES))?;
    let doctors = client
        .schedule_doctors(&session.token, &session.clinic_id)
        .await
        .map_err(|error| error.message)?;
    let selected = resolve_doctor(&doctors, &session.doctor_id, doctor.as_deref())?;
    let room_ids = selected
        .schedules
        .iter()
        .map(|schedule| schedule.treatment_room_id.clone())
        .collect::<std::collections::BTreeSet<_>>()
        .into_iter()
        .collect::<Vec<_>>();
    let appointments = schedule_occupancy(
        &client,
        &session,
        &selected.id,
        &room_ids,
        &from_date.to_string(),
        &to_date.to_string(),
    )
    .await?;
    let slot = find_first_available(
        selected,
        &appointments,
        from_date,
        to_date,
        after_time,
        duration_minutes,
        now,
    )?;
    state.aidoo.store_schedule_slot(slot.clone())?;
    show_schedule_view(&app, &state, &slot);
    Ok(slot)
}

#[tauri::command]
pub(crate) async fn aidoo_book_schedule_slot(
    slot_id: String,
    patient_query: Option<String>,
    patient_id: Option<String>,
    app: AppHandle,
    state: State<'_, AppState>,
) -> Result<types::ScheduleBookingResult, String> {
    let slot = state.aidoo.schedule_slot(&slot_id)?;
    let session = state.aidoo.session()?;
    let client = state.aidoo.client()?;
    let patient = match resolve_booking_patient(
        &state,
        &client,
        &session,
        patient_query.as_deref(),
        patient_id.as_deref(),
    )
    .await?
    {
        BookingPatient::Selected(patient) => patient,
        BookingPatient::NeedsSelection(matches) => {
            return Ok(types::ScheduleBookingResult {
                booked: false,
                needs_patient_selection: true,
                matches,
                slot,
                verification: None,
                spoken_summary: "Намерих няколко пациенти. Уточнете кой да бъде записан.".into(),
            });
        }
    };

    let day = NaiveDate::parse_from_str(&slot.local_date, "%Y-%m-%d")
        .map_err(|_| "Предложеният час съдържа невалидна дата.".to_string())?;
    let doctors = client
        .schedule_doctors(&session.token, &session.clinic_id)
        .await
        .map_err(|error| error.message)?;
    if !slot_is_in_current_worktime(&slot, &doctors)? {
        state.aidoo.clear_schedule_slot();
        show_schedule_view(&app, &state, &slot);
        return Err(
            "Работният график за предложения час е променен. Потърсете свободен час отново.".into(),
        );
    }
    let fresh = schedule_occupancy(
        &client,
        &session,
        &slot.doctor_id,
        std::slice::from_ref(&slot.treatment_room_id),
        &day.to_string(),
        &day.to_string(),
    )
    .await?;
    if slot_conflicts(&slot, &fresh)? {
        state.aidoo.clear_schedule_slot();
        show_schedule_view(&app, &state, &slot);
        return Err("Часът вече е зает. Потърсете свободен час отново.".into());
    }

    let request = types::CreateAppointmentRequest {
        appointment_type: "PATIENT",
        doctor_id: &slot.doctor_id,
        treatment_room_id: &slot.treatment_room_id,
        start_time: &slot.start_time,
        end_time: &slot.end_time,
        patient_appointment: types::CreateAppointmentPatient {
            patient_id: &patient.id,
            first_name: &patient.first_name,
            middle_name: patient.middle_name.as_deref(),
            last_name: &patient.last_name,
            mobile_phone: patient.mobile_phone.as_deref(),
            status: "SCHEDULED",
            dental_technology_readiness: "NOT_SET",
        },
    };
    let write_result = client
        .create_appointment(&session.token, &session.clinic_id, &request)
        .await;
    let doctor_ids = vec![slot.doctor_id.clone()];
    let read_back = client
        .search_appointments(
            &session.token,
            &session.clinic_id,
            Some(&doctor_ids),
            None,
            &day.to_string(),
            &day.to_string(),
        )
        .await;
    let verified = read_back.as_ref().is_ok_and(|appointments| {
        appointments.iter().any(|appointment| {
            appointment.start_time == slot.start_time
                && appointment.end_time == slot.end_time
                && appointment
                    .patient_appointment
                    .as_ref()
                    .and_then(|value| value.patient_id.as_deref())
                    == Some(patient.id.as_str())
        })
    });

    let verification = match (&write_result, verified) {
        (Ok(()), true) => types::VerificationResult {
            outcome: types::VerificationOutcome::Verified,
            message: "Часът е записан и проверен в графика.".into(),
        },
        (Err(error), true) if error.is_ambiguous_write() => types::VerificationResult {
            outcome: types::VerificationOutcome::VerifiedAfterAmbiguousWrite,
            message: "Връзката прекъсна, но часът е намерен при независимата проверка.".into(),
        },
        (Err(error), false) if !error.is_ambiguous_write() => {
            state.aidoo.clear_schedule_slot();
            show_schedule_view(&app, &state, &slot);
            return Err(error.message.clone());
        }
        _ => types::VerificationResult {
            outcome: types::VerificationOutcome::Uncertain,
            message: "Не мога да потвърдя дали часът е записан. Проверете показания график; няма да повтарям записа автоматично.".into(),
        },
    };
    let booked = matches!(
        verification.outcome,
        types::VerificationOutcome::Verified
            | types::VerificationOutcome::VerifiedAfterAmbiguousWrite
    );
    state.aidoo.clear_schedule_slot();
    show_schedule_view(&app, &state, &slot);
    let spoken_summary = if booked {
        format!(
            "Записах пациента на {} от {} за {} минути.",
            format_bg_date(&slot.local_date),
            slot.local_time,
            slot.duration_minutes
        )
    } else {
        verification.message.clone()
    };
    Ok(types::ScheduleBookingResult {
        booked,
        needs_patient_selection: false,
        matches: Vec::new(),
        slot,
        verification: Some(verification),
        spoken_summary,
    })
}

async fn schedule_occupancy(
    client: &AidooClient,
    session: &AidooSessionSnapshot,
    doctor_id: &str,
    room_ids: &[String],
    from_date: &str,
    to_date: &str,
) -> Result<Vec<types::ScheduleAppointment>, String> {
    if room_ids.is_empty() {
        return Err("Лекарят няма кабинет в работния си график.".into());
    }
    let doctor_ids = vec![doctor_id.to_string()];
    let (doctor_appointments, room_appointments) = tokio::try_join!(
        client.search_appointments(
            &session.token,
            &session.clinic_id,
            Some(&doctor_ids),
            None,
            from_date,
            to_date,
        ),
        client.search_appointments(
            &session.token,
            &session.clinic_id,
            None,
            Some(room_ids),
            from_date,
            to_date,
        )
    )
    .map_err(|error| error.message)?;
    let mut merged = doctor_appointments;
    for appointment in room_appointments {
        if !merged.iter().any(|known| known.id == appointment.id) {
            merged.push(appointment);
        }
    }
    Ok(merged)
}

fn show_schedule_view(app: &AppHandle, state: &AppState, slot: &types::ScheduleSlot) {
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
        presentation::present_schedule(
            app.clone(),
            clinic_link,
            slot.local_date.clone(),
            slot.doctor_id.clone(),
        );
    }
}

enum BookingPatient {
    Selected(types::PatientSummary),
    NeedsSelection(Vec<types::PatientSummary>),
}

async fn resolve_booking_patient(
    state: &AppState,
    client: &AidooClient,
    session: &AidooSessionSnapshot,
    patient_query: Option<&str>,
    patient_id: Option<&str>,
) -> Result<BookingPatient, String> {
    match (
        patient_query
            .map(str::trim)
            .filter(|value| !value.is_empty()),
        patient_id,
    ) {
        (Some(_), Some(_)) => {
            Err("Подайте име на пациент или избран patientId, но не и двете.".into())
        }
        (None, Some(id)) => state.aidoo.recent_patient(id).map(BookingPatient::Selected),
        (Some(query), None) => {
            let results = client
                .search_patients(&session.token, &session.clinic_id, query)
                .await
                .map_err(|error| error.message)?;
            let selected = state.aidoo.remember_patient_search(&results)?;
            if let Some(patient) = selected {
                return Ok(BookingPatient::Selected(patient));
            }
            let matches = results
                .into_iter()
                .map(|result| result.patient)
                .collect::<Vec<_>>();
            if matches.is_empty() {
                Err("Не е намерен пациент за записване в графика.".into())
            } else {
                Ok(BookingPatient::NeedsSelection(matches))
            }
        }
        (None, None) => Err("Кажете кой пациент да бъде записан в предложения час.".into()),
    }
}

fn search_dates(value: Option<&str>, today: NaiveDate) -> Result<(NaiveDate, NaiveDate), String> {
    if let Some(value) = value.map(str::trim).filter(|value| !value.is_empty()) {
        let date = NaiveDate::parse_from_str(value, "%Y-%m-%d")
            .map_err(|_| "Датата трябва да бъде във формат YYYY-MM-DD.".to_string())?;
        if date < today {
            return Err("Не може да се търси свободен час в миналото.".into());
        }
        return Ok((date, date));
    }
    Ok((today, today + Duration::days(SEARCH_HORIZON_DAYS)))
}

fn parse_time(value: &str) -> Result<NaiveTime, String> {
    NaiveTime::parse_from_str(value.trim(), "%H:%M")
        .or_else(|_| NaiveTime::parse_from_str(value.trim(), "%H:%M:%S"))
        .map_err(|_| "Часът трябва да бъде във формат HH:MM.".into())
}

fn validate_duration(value: i64) -> Result<i64, String> {
    if (15..=240).contains(&value) && value % SLOT_STEP_MINUTES == 0 {
        Ok(value)
    } else {
        Err("Продължителността трябва да е между 15 и 240 минути, през 15 минути.".into())
    }
}

fn normalized(value: &str) -> String {
    value
        .to_lowercase()
        .replace(|character: char| !character.is_alphanumeric(), " ")
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ")
}

fn resolve_doctor<'a>(
    doctors: &'a [types::ScheduleDoctor],
    signed_in_doctor_id: &str,
    query: Option<&str>,
) -> Result<&'a types::ScheduleDoctor, String> {
    let doctors = doctors
        .iter()
        .filter(|doctor| doctor.doctor)
        .collect::<Vec<_>>();
    if let Some(query) = query.map(normalized).filter(|value| !value.is_empty()) {
        let matches = doctors
            .iter()
            .copied()
            .filter(|doctor| {
                let full = normalized(&format!("{} {}", doctor.first_name, doctor.last_name));
                full == query || full.contains(&query)
            })
            .collect::<Vec<_>>();
        return match matches.as_slice() {
            [doctor] => Ok(*doctor),
            [] => Err("Лекарят не е намерен в графика на клиниката.".into()),
            _ => Err("Името на лекаря е двусмислено. Уточнете пълното име.".into()),
        };
    }
    doctors
        .into_iter()
        .find(|doctor| doctor.id == signed_in_doctor_id)
        .ok_or_else(|| "Свързаният AIDOO профил няма лекарски график.".into())
}

fn find_first_available(
    doctor: &types::ScheduleDoctor,
    appointments: &[types::ScheduleAppointment],
    from_date: NaiveDate,
    to_date: NaiveDate,
    after_time: NaiveTime,
    duration_minutes: i64,
    now: DateTime<Local>,
) -> Result<types::ScheduleSlot, String> {
    let mut date = from_date;
    while date <= to_date {
        let weekday = weekday_name(date.weekday());
        let mut candidates = Vec::new();
        for schedule in doctor
            .schedules
            .iter()
            .filter(|schedule| schedule.weekday == weekday)
        {
            let schedule_start = parse_time(&schedule.start_time)?;
            let schedule_end = parse_time(&schedule.end_time)?;
            let mut earliest = schedule_start.max(after_time);
            if date == now.date_naive() {
                earliest = earliest.max(now.time());
            }
            let mut cursor = round_up_quarter(date.and_time(earliest))?;
            let local_end = local_datetime(date.and_time(schedule_end))?;
            while cursor + Duration::minutes(duration_minutes) <= local_end {
                let end = cursor + Duration::minutes(duration_minutes);
                let start_utc = cursor.with_timezone(&Utc);
                let end_utc = end.with_timezone(&Utc);
                if !appointments.iter().any(|appointment| {
                    appointment_conflicts(
                        appointment,
                        &doctor.id,
                        &schedule.treatment_room_id,
                        start_utc,
                        end_utc,
                    )
                    .unwrap_or(true)
                }) {
                    candidates.push((cursor, end, schedule));
                    break;
                }
                cursor += Duration::minutes(SLOT_STEP_MINUTES);
            }
        }
        if let Some((start, end, schedule)) = candidates.into_iter().min_by_key(|value| value.0) {
            return Ok(types::ScheduleSlot {
                id: uuid::Uuid::new_v4().to_string(),
                doctor_id: doctor.id.clone(),
                doctor_name: format!("{} {}", doctor.first_name, doctor.last_name),
                treatment_room_id: schedule.treatment_room_id.clone(),
                start_time: start
                    .with_timezone(&Utc)
                    .to_rfc3339_opts(chrono::SecondsFormat::Secs, true),
                end_time: end
                    .with_timezone(&Utc)
                    .to_rfc3339_opts(chrono::SecondsFormat::Secs, true),
                local_date: start.date_naive().to_string(),
                local_time: start.format("%H:%M").to_string(),
                duration_minutes,
            });
        }
        date += Duration::days(1);
    }
    Err("Не е намерен свободен час в избрания период, работното време и продължителност.".into())
}

fn round_up_quarter(value: NaiveDateTime) -> Result<DateTime<Local>, String> {
    let minute = value.minute() as i64;
    let seconds = value.second() as i64;
    let remainder = minute % SLOT_STEP_MINUTES;
    let add = if remainder == 0 && seconds == 0 {
        0
    } else {
        SLOT_STEP_MINUTES - remainder
    };
    local_datetime(
        value
            .with_second(0)
            .and_then(|value| value.with_nanosecond(0))
            .unwrap_or(value)
            + Duration::minutes(add),
    )
}

fn local_datetime(value: NaiveDateTime) -> Result<DateTime<Local>, String> {
    Local
        .from_local_datetime(&value)
        .single()
        .ok_or_else(|| "Часът попада в невалидна смяна на часовото време.".into())
}

fn appointment_conflicts(
    appointment: &types::ScheduleAppointment,
    doctor_id: &str,
    treatment_room_id: &str,
    start: DateTime<Utc>,
    end: DateTime<Utc>,
) -> Result<bool, String> {
    if appointment.doctor.id != doctor_id && appointment.treatment_room_id != treatment_room_id {
        return Ok(false);
    }
    let occupied_start = DateTime::parse_from_rfc3339(&appointment.start_time)
        .map_err(|_| "AIDOO върна невалидно начало на час.".to_string())?
        .with_timezone(&Utc);
    let occupied_end = DateTime::parse_from_rfc3339(&appointment.end_time)
        .map_err(|_| "AIDOO върна невалиден край на час.".to_string())?
        .with_timezone(&Utc);
    Ok(start < occupied_end && end > occupied_start)
}

fn slot_conflicts(
    slot: &types::ScheduleSlot,
    appointments: &[types::ScheduleAppointment],
) -> Result<bool, String> {
    let start = DateTime::parse_from_rfc3339(&slot.start_time)
        .map_err(|_| "Предложеният час съдържа невалидно начало.".to_string())?
        .with_timezone(&Utc);
    let end = DateTime::parse_from_rfc3339(&slot.end_time)
        .map_err(|_| "Предложеният час съдържа невалиден край.".to_string())?
        .with_timezone(&Utc);
    appointments
        .iter()
        .try_fold(false, |conflict, appointment| {
            Ok(conflict
                || appointment_conflicts(
                    appointment,
                    &slot.doctor_id,
                    &slot.treatment_room_id,
                    start,
                    end,
                )?)
        })
}

fn slot_is_in_current_worktime(
    slot: &types::ScheduleSlot,
    doctors: &[types::ScheduleDoctor],
) -> Result<bool, String> {
    let start = DateTime::parse_from_rfc3339(&slot.start_time)
        .map_err(|_| "Предложеният час съдържа невалидно начало.".to_string())?
        .with_timezone(&Local);
    let end = DateTime::parse_from_rfc3339(&slot.end_time)
        .map_err(|_| "Предложеният час съдържа невалиден край.".to_string())?
        .with_timezone(&Local);
    let Some(doctor) = doctors.iter().find(|doctor| doctor.id == slot.doctor_id) else {
        return Ok(false);
    };
    let weekday = weekday_name(start.weekday());
    doctor.schedules.iter().try_fold(false, |valid, schedule| {
        if valid
            || schedule.weekday != weekday
            || schedule.treatment_room_id != slot.treatment_room_id
        {
            return Ok(valid);
        }
        let work_start = parse_time(&schedule.start_time)?;
        let work_end = parse_time(&schedule.end_time)?;
        Ok(start.date_naive() == end.date_naive()
            && start.time() >= work_start
            && end.time() <= work_end)
    })
}

fn weekday_name(value: Weekday) -> &'static str {
    match value {
        Weekday::Mon => "MONDAY",
        Weekday::Tue => "TUESDAY",
        Weekday::Wed => "WEDNESDAY",
        Weekday::Thu => "THURSDAY",
        Weekday::Fri => "FRIDAY",
        Weekday::Sat => "SATURDAY",
        Weekday::Sun => "SUNDAY",
    }
}

fn format_bg_date(value: &str) -> String {
    NaiveDate::parse_from_str(value, "%Y-%m-%d")
        .map(|date| date.format("%d.%m.%Y").to_string())
        .unwrap_or_else(|_| value.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn doctor() -> types::ScheduleDoctor {
        types::ScheduleDoctor {
            id: "doctor-1".into(),
            first_name: "Тест".into(),
            last_name: "Лекар".into(),
            doctor: true,
            schedules: vec![types::DoctorSchedule {
                treatment_room_id: "room-1".into(),
                weekday: "MONDAY".into(),
                start_time: "09:00:00".into(),
                end_time: "17:00:00".into(),
            }],
        }
    }

    fn appointment(start: &str, end: &str) -> types::ScheduleAppointment {
        types::ScheduleAppointment {
            id: "appointment-1".into(),
            doctor: types::AppointmentDoctor {
                id: "doctor-1".into(),
            },
            treatment_room_id: "room-1".into(),
            start_time: start.into(),
            end_time: end.into(),
            patient_appointment: None,
        }
    }

    #[test]
    fn finds_the_first_quarter_hour_gap_after_the_requested_time() {
        let date = NaiveDate::from_ymd_opt(2026, 9, 21).unwrap();
        let occupied_start = local_datetime(date.and_hms_opt(9, 0, 0).unwrap())
            .unwrap()
            .with_timezone(&Utc);
        let occupied = appointment(
            &occupied_start.to_rfc3339(),
            &(occupied_start + Duration::hours(1)).to_rfc3339(),
        );
        let now = Local
            .from_local_datetime(&date.and_hms_opt(8, 0, 0).unwrap())
            .single()
            .unwrap();
        let slot = find_first_available(
            &doctor(),
            &[occupied],
            date,
            date,
            NaiveTime::from_hms_opt(12, 7, 0).unwrap(),
            30,
            now,
        )
        .unwrap();
        assert_eq!(slot.local_date, "2026-09-21");
        assert_eq!(slot.local_time, "12:15");
        assert_eq!(slot.duration_minutes, 30);
    }

    #[test]
    fn blocks_overlap_for_the_doctor_or_the_room() -> Result<(), String> {
        let date = NaiveDate::from_ymd_opt(2026, 9, 21).unwrap();
        let start = local_datetime(date.and_hms_opt(12, 0, 0).unwrap())?.with_timezone(&Utc);
        let end = start + Duration::minutes(30);
        let occupied = appointment(&start.to_rfc3339(), &end.to_rfc3339());
        assert!(appointment_conflicts(
            &occupied,
            "doctor-1",
            "other-room",
            start,
            end
        )?);
        assert!(appointment_conflicts(
            &occupied,
            "other-doctor",
            "room-1",
            start,
            end
        )?);
        assert!(!appointment_conflicts(
            &occupied,
            "other-doctor",
            "other-room",
            start,
            end
        )?);
        Ok::<(), String>(())
    }

    #[test]
    fn explicit_date_does_not_spill_into_the_next_day() {
        let today = NaiveDate::from_ymd_opt(2026, 9, 21).unwrap();
        assert_eq!(
            search_dates(Some("2026-09-24"), today).unwrap(),
            (
                NaiveDate::from_ymd_opt(2026, 9, 24).unwrap(),
                NaiveDate::from_ymd_opt(2026, 9, 24).unwrap(),
            )
        );
    }
}
