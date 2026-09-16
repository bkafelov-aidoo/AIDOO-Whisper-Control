use super::{scripted_server, ResponseScript};
use crate::aidoo::{client::AidooClient, types::*};
use std::time::Duration;

#[tokio::test]
async fn client_uses_the_observed_schedule_search_and_create_contracts() {
    let (base, requests) = scripted_server(vec![
        ResponseScript::json(
            200,
            r#"[{"id":"doctor-id","firstName":"Тест","lastName":"Лекар","doctor":true,"schedules":[{"treatmentRoomId":"room-id","weekday":"MONDAY","startTime":"09:00:00","endTime":"17:00:00"}]}]"#,
        ),
        ResponseScript::json(200, "[]"),
        ResponseScript::json(201, "{}"),
    ]);
    let client = AidooClient::for_test(base, Duration::from_secs(2));
    let doctors = client.schedule_doctors("token", "clinic-id").await.unwrap();
    assert_eq!(doctors[0].schedules[0].weekday, "MONDAY");
    let doctor_ids = vec!["doctor-id".into()];
    client
        .search_appointments(
            "token",
            "clinic-id",
            Some(&doctor_ids),
            None,
            "2026-09-21",
            "2026-09-21",
        )
        .await
        .unwrap();
    client
        .create_appointment(
            "token",
            "clinic-id",
            &CreateAppointmentRequest {
                appointment_type: "PATIENT",
                doctor_id: "doctor-id",
                treatment_room_id: "room-id",
                start_time: "2026-09-21T09:00:00Z",
                end_time: "2026-09-21T09:30:00Z",
                patient_appointment: CreateAppointmentPatient {
                    patient_id: "patient-id",
                    first_name: "Тест",
                    middle_name: None,
                    last_name: "Пациент",
                    mobile_phone: None,
                    status: "SCHEDULED",
                    dental_technology_readiness: "NOT_SET",
                },
            },
        )
        .await
        .unwrap();

    let captured = (0..3)
        .map(|_| requests.recv_timeout(Duration::from_secs(1)).unwrap())
        .collect::<Vec<_>>();
    assert!(captured[0].starts_with("GET /clinics/clinic-id/users HTTP/1.1"));
    assert!(captured[1].starts_with("POST /clinics/clinic-id/appointments/search HTTP/1.1"));
    assert!(captured[1].contains("\"doctorIds\":[\"doctor-id\"]"));
    assert!(captured[1].contains("\"treatmentRoomIds\":null"));
    assert!(captured[2].starts_with("POST /clinics/clinic-id/appointments HTTP/1.1"));
    assert!(captured[2].contains("\"appointmentType\":\"PATIENT\""));
    assert!(captured[2].contains("\"patientId\":\"patient-id\""));
    assert!(captured[2].contains("\"dentalTechnologyReadiness\":\"NOT_SET\""));
}
