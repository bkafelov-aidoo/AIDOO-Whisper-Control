use super::client::AidooClient;
use super::draft::{build_draft, editable_status_catalog, verifies};
use super::treatment::{build_treatment_draft, same_treatment_snapshot, verifies_treatment};
use super::types::*;
use super::workflow::{
    apply_confirmed_draft, apply_confirmed_treatment_draft, create_status_visit,
};
use std::io::{Read, Write};
use std::net::{TcpListener, TcpStream};
use std::sync::{mpsc, Arc, Mutex};
use std::thread;
use std::time::Duration;

mod edge_cases;
mod schedule_contract;

fn catalog() -> Vec<StatusCatalogEntry> {
    vec![
        StatusCatalogEntry {
            id: "caries-id".into(),
            name: "Кариес".into(),
            code: "C".into(),
            order: 1,
            diagnosis_id: None,
            can_have_regions: true,
            regions: vec![
                "MESIAL".into(),
                "DISTAL".into(),
                "OCCLUSAL".into(),
                "VESTIBULAR".into(),
                "LINGUAL".into(),
                "CERVICAL_LINGUAL".into(),
                "CERVICAL_VESTIBULAR".into(),
            ],
            incompatible_statuses: vec![],
            nzis_tooth_diagnosis_id: Some("nzis-caries".into()),
        },
        StatusCatalogEntry {
            id: "restoration-id".into(),
            name: "Обтурация".into(),
            code: "O".into(),
            order: 2,
            diagnosis_id: None,
            can_have_regions: true,
            regions: vec![
                "MESIAL".into(),
                "DISTAL".into(),
                "OCCLUSAL".into(),
                "VESTIBULAR".into(),
                "LINGUAL".into(),
                "CERVICAL_LINGUAL".into(),
                "CERVICAL_VESTIBULAR".into(),
            ],
            incompatible_statuses: vec![],
            nzis_tooth_diagnosis_id: Some("nzis-restoration".into()),
        },
    ]
}

fn status(tooth: &str, statuses: &[&str], regions: &[&str]) -> ToothStatus {
    ToothStatus {
        id: Some(format!("record-{tooth}")),
        tooth: tooth.into(),
        statuses: statuses.iter().map(|value| (*value).into()).collect(),
        is_milk_tooth: false,
        for_observation: false,
        regions: regions.iter().map(|value| (*value).into()).collect(),
        timestamp: Some("2026-09-16T00:00:00Z".into()),
        note: None,
        generated_by_procedure: false,
    }
}

fn visit(created_status_update: bool) -> Visit {
    Visit {
        id: "visit-id".into(),
        created_status_update,
        is_finished: false,
        cancelled: false,
    }
}

#[test]
fn surface_add_builds_the_observed_aidoo_payload() {
    let draft = build_draft(
        "patient-id".into(),
        &visit(true),
        false,
        vec![status("32", &[], &[])],
        &catalog(),
        &[StatusChange {
            operation: StatusOperation::Add,
            tooth: "32".into(),
            status_id: "caries-id".into(),
            regions: vec!["OCCLUSAL".into()],
            existing_status_id: None,
            is_milk_tooth: false,
            for_observation: false,
            note: None,
        }],
    )
    .unwrap();

    assert_eq!(draft.writes.len(), 1);
    assert_eq!(draft.writes[0].tooth, "32");
    assert_eq!(draft.writes[0].statuses, ["caries-id"]);
    assert_eq!(draft.writes[0].regions, ["OCCLUSAL"]);
    assert!(!draft.writes[0].is_milk_tooth);
    assert!(!draft.writes[0].for_observation);
    assert!(draft.spoken_summary.contains("зъб 32"));
}

#[test]
fn assistant_catalog_excludes_nzis_surface_mapping_entries() {
    let entries = vec![
        StatusCatalogEntry {
            id: "caries-id".into(),
            name: "Кариес".into(),
            code: "C".into(),
            order: 1,
            diagnosis_id: None,
            can_have_regions: true,
            regions: vec!["MESIAL".into(), "OCCLUSAL".into()],
            incompatible_statuses: vec![],
            nzis_tooth_diagnosis_id: Some("nzis-caries".into()),
        },
        StatusCatalogEntry {
            id: "nzis-occlusal-caries-id".into(),
            name: "Кариес (Оклузално / Инцизално / Куспидално)".into(),
            code: "Co".into(),
            order: 30,
            diagnosis_id: None,
            can_have_regions: true,
            regions: vec![],
            incompatible_statuses: vec![],
            nzis_tooth_diagnosis_id: Some("nzis-occlusal-caries".into()),
        },
    ];

    let filtered = editable_status_catalog(entries);
    assert_eq!(filtered.len(), 1);
    assert_eq!(filtered[0].id, "caries-id");
}

#[test]
fn surface_protocol_requires_base_status_and_supported_regions() {
    let catalog = vec![StatusCatalogEntry {
        id: "caries-id".into(),
        name: "Кариес".into(),
        code: "C".into(),
        order: 1,
        diagnosis_id: None,
        can_have_regions: true,
        regions: vec!["MESIAL".into(), "OCCLUSAL".into(), "LINGUAL".into()],
        incompatible_statuses: vec![],
        nzis_tooth_diagnosis_id: Some("nzis-caries".into()),
    }];

    let no_surface = StatusChange {
        operation: StatusOperation::Add,
        tooth: "16".into(),
        status_id: "caries-id".into(),
        regions: vec![],
        existing_status_id: None,
        is_milk_tooth: false,
        for_observation: false,
        note: None,
    };
    assert!(build_draft(
        "patient-id".into(),
        &visit(true),
        false,
        vec![],
        &catalog,
        &[no_surface],
    )
    .unwrap_err()
    .contains("повърхност"));

    let unsupported = StatusChange {
        operation: StatusOperation::Add,
        tooth: "16".into(),
        status_id: "caries-id".into(),
        regions: vec!["CERVICAL_VESTIBULAR".into()],
        existing_status_id: None,
        is_milk_tooth: false,
        for_observation: false,
        note: None,
    };
    assert!(build_draft(
        "patient-id".into(),
        &visit(true),
        false,
        vec![],
        &catalog,
        &[unsupported],
    )
    .is_err());

    let palatal = StatusChange {
        operation: StatusOperation::Add,
        tooth: "16".into(),
        status_id: "caries-id".into(),
        regions: vec!["PALATAL".into()],
        existing_status_id: None,
        is_milk_tooth: false,
        for_observation: false,
        note: None,
    };
    let draft = build_draft(
        "patient-id".into(),
        &visit(true),
        false,
        vec![],
        &catalog,
        &[palatal],
    )
    .unwrap();
    assert_eq!(draft.writes[0].regions, ["LINGUAL"]);
}

#[test]
fn incompatible_statuses_are_rejected_across_surface_rows_of_the_same_tooth() {
    let mut catalog = catalog();
    catalog[0].incompatible_statuses = vec!["restoration-id".into()];
    let change = StatusChange {
        operation: StatusOperation::Add,
        tooth: "16".into(),
        status_id: "caries-id".into(),
        regions: vec!["OCCLUSAL".into()],
        existing_status_id: None,
        is_milk_tooth: false,
        for_observation: false,
        note: None,
    };

    let error = build_draft(
        "patient-id".into(),
        &visit(true),
        false,
        vec![status("16", &["restoration-id"], &["MESIAL"])],
        &catalog,
        &[change],
    )
    .unwrap_err();
    assert!(error.contains("несъвместими"));
}

#[test]
fn replace_and_multiple_changes_are_aggregated_without_losing_other_statuses() {
    let baseline = vec![
        status("32", &["restoration-id"], &["OCCLUSAL"]),
        status("31", &[], &[]),
    ];
    let draft = build_draft(
        "patient-id".into(),
        &visit(true),
        false,
        baseline,
        &catalog(),
        &[
            StatusChange {
                operation: StatusOperation::Replace,
                tooth: "32".into(),
                status_id: "caries-id".into(),
                regions: vec!["OCCLUSAL".into()],
                existing_status_id: Some("restoration-id".into()),
                is_milk_tooth: false,
                for_observation: false,
                note: None,
            },
            StatusChange {
                operation: StatusOperation::Add,
                tooth: "31".into(),
                status_id: "caries-id".into(),
                regions: vec!["MESIAL".into()],
                existing_status_id: None,
                is_milk_tooth: false,
                for_observation: false,
                note: Some("контролен запис".into()),
            },
        ],
    )
    .unwrap();

    assert_eq!(draft.writes.len(), 2);
    assert!(draft.writes.iter().any(|write| {
        write.tooth == "32" && write.statuses == ["caries-id"] && write.regions == ["OCCLUSAL"]
    }));
    assert!(draft.writes.iter().any(|write| {
        write.tooth == "31"
            && write.statuses == ["caries-id"]
            && write.regions == ["MESIAL"]
            && write.note.as_deref() == Some("контролен запис")
    }));
}

#[test]
fn draft_rejects_unknown_status_surface_and_stale_replacement() {
    for change in [
        StatusChange {
            operation: StatusOperation::Add,
            tooth: "32".into(),
            status_id: "unknown".into(),
            regions: vec![],
            existing_status_id: None,
            is_milk_tooth: false,
            for_observation: false,
            note: None,
        },
        StatusChange {
            operation: StatusOperation::Add,
            tooth: "32".into(),
            status_id: "caries-id".into(),
            regions: vec!["TOP".into()],
            existing_status_id: None,
            is_milk_tooth: false,
            for_observation: false,
            note: None,
        },
        StatusChange {
            operation: StatusOperation::Replace,
            tooth: "32".into(),
            status_id: "caries-id".into(),
            regions: vec!["OCCLUSAL".into()],
            existing_status_id: Some("restoration-id".into()),
            is_milk_tooth: false,
            for_observation: false,
            note: None,
        },
    ] {
        assert!(build_draft(
            "patient-id".into(),
            &visit(true),
            false,
            vec![status("32", &[], &[])],
            &catalog(),
            &[change],
        )
        .is_err());
    }
}

#[test]
fn verification_requires_the_exact_status_set_for_the_surface() {
    let expected = ToothStatusWrite {
        tooth: "32".into(),
        statuses: vec!["caries-id".into()],
        is_milk_tooth: false,
        for_observation: false,
        regions: vec!["OCCLUSAL".into()],
        note: None,
    };
    assert!(verifies(
        std::slice::from_ref(&expected),
        &[status("32", &["caries-id"], &["OCCLUSAL"])]
    ));
    assert!(!verifies(
        &[expected],
        &[status(
            "32",
            &["caries-id", "restoration-id"],
            &["OCCLUSAL"]
        )]
    ));
}

#[test]
fn treatment_draft_validates_catalogs_and_preserves_the_dictated_official_note() {
    let baseline = vec![treatment("treatment-row", None, None, None, &[])];
    let draft = build_treatment_draft(
        "patient-id".into(),
        &visit(true),
        baseline,
        &diagnosis_catalog(),
        &procedure_catalog(),
        TreatmentChange {
            tooth: "26".into(),
            existing_treatment_id: Some("treatment-row".into()),
            diagnosis_id: Some("diagnosis-id".into()),
            treatment_id: None,
            note: Some("  Пациентът е информиран.  ".into()),
            procedure_ids: vec!["procedure-id".into()],
        },
    )
    .unwrap();

    assert_eq!(
        draft.treatment.diagnosis_id.as_deref(),
        Some("diagnosis-id")
    );
    assert_eq!(
        draft.treatment.note.as_deref(),
        Some("Пациентът е информиран.")
    );
    assert_eq!(draft.procedures[0].procedure_id, "procedure-id");
    assert_eq!(draft.procedures[0].price, "42.5");
    assert!(draft.spoken_summary.contains("официална забележка"));
    assert!(verifies_treatment(
        &draft,
        &[treatment(
            "treatment-row",
            Some("diagnosis-id"),
            Some("Пациентът е информиран."),
            None,
            &["procedure-id"]
        )]
    ));
}

#[test]
fn treatment_draft_rejects_unknown_diagnoses_and_procedures() {
    for change in [
        TreatmentChange {
            tooth: "26".into(),
            existing_treatment_id: None,
            diagnosis_id: Some("unknown".into()),
            treatment_id: None,
            note: None,
            procedure_ids: vec![],
        },
        TreatmentChange {
            tooth: "26".into(),
            existing_treatment_id: None,
            diagnosis_id: None,
            treatment_id: None,
            note: None,
            procedure_ids: vec!["unknown".into()],
        },
    ] {
        assert!(build_treatment_draft(
            "patient-id".into(),
            &visit(true),
            vec![],
            &diagnosis_catalog(),
            &procedure_catalog(),
            change,
        )
        .is_err());
    }
}

#[tokio::test]
async fn client_uses_the_observed_put_route_query_and_body() {
    let (base, requests) =
        scripted_server(vec![ResponseScript::json(200, r#"{"teethStatus":[]}"#)]);
    let client = AidooClient::for_test(base, Duration::from_secs(2));
    let write = ToothStatusWrite {
        tooth: "32".into(),
        statuses: vec!["caries-id".into()],
        is_milk_tooth: false,
        for_observation: false,
        regions: vec!["OCCLUSAL".into()],
        note: None,
    };

    client
        .write_status(
            "session-token",
            "clinic-id",
            "patient-id",
            "visit-id",
            &[write],
        )
        .await
        .unwrap();

    let request = requests.recv_timeout(Duration::from_secs(1)).unwrap();
    assert!(request.starts_with(
        "PUT /clinics/clinic-id/patients/patient-id/teeth-status?visitId=visit-id HTTP/1.1"
    ));
    assert!(request
        .to_ascii_lowercase()
        .contains("x-auth-token: session-token"));
    assert!(request.contains("\"teethStatus\":[{"));
    assert!(request.contains("\"regions\":[\"OCCLUSAL\"]"));
    assert!(!request.contains("\"id\""));
}

#[tokio::test]
async fn client_uses_the_observed_create_status_update_route() {
    let (base, requests) =
        scripted_server(vec![ResponseScript::json(200, r#"{"teethStatus":[]}"#)]);
    let client = AidooClient::for_test(base, Duration::from_secs(2));

    client
        .create_status_update(
            "session-token",
            "clinic-id",
            "patient-id",
            "visit-id",
            false,
        )
        .await
        .unwrap();

    let request = requests.recv_timeout(Duration::from_secs(2)).unwrap();
    assert!(request.starts_with(
        "POST /clinics/clinic-id/patients/patient-id/teeth-status?visitId=visit-id&isNzok=false HTTP/1.1"
    ));
    assert!(request
        .to_ascii_lowercase()
        .contains("x-auth-token: session-token"));
}

#[tokio::test]
async fn client_uses_observed_diagnosis_procedure_treatment_and_note_routes() {
    let treatment_json =
        treatment_json("treatment-row", Some("diagnosis-id"), Some("Бележка"), &[]);
    let procedure_response = serde_json::json!({
        "procedure": {"id":"joined-id","procedureId":"procedure-id","price":"42.5","discount":"0"},
        "treatmentId":"treatment-row"
    })
    .to_string();
    let (base, requests) = scripted_server(vec![
        ResponseScript::json(
            200,
            r#"[{"id":"diagnosis-id","name":"Кариес","key":"K02"}]"#,
        ),
        ResponseScript::json(
            200,
            r#"[{"id":"procedure-id","name":"Обтурация","key":"P1","price":"42.5"}]"#,
        ),
        ResponseScript::json(200, &format!("[{treatment_json}]")),
        ResponseScript::json(200, &treatment_json),
        ResponseScript::json(200, &procedure_response),
    ]);
    let client = AidooClient::for_test(base, Duration::from_secs(2));
    client
        .diagnosis_catalog("token", "clinic-id")
        .await
        .unwrap();
    client
        .procedure_catalog("token", "clinic-id", Some("EUR"))
        .await
        .unwrap();
    client
        .visit_treatments("token", "clinic-id", "patient-id", "visit-id")
        .await
        .unwrap();
    let write = TreatmentWrite {
        tooth: "26".into(),
        diagnosis_id: Some("diagnosis-id".into()),
        treatment_id: None,
        note: Some("Бележка".into()),
        status: None,
        is_milk_tooth: false,
    };
    client
        .update_treatment(
            "token",
            "clinic-id",
            "patient-id",
            "visit-id",
            "treatment-row",
            &write,
        )
        .await
        .unwrap();
    client
        .add_procedure(
            "token",
            "clinic-id",
            "patient-id",
            "treatment-row",
            &ProcedureWrite {
                id: None,
                procedure_id: "procedure-id".into(),
                price: "42.5".into(),
                discount: "0".into(),
            },
        )
        .await
        .unwrap();

    let captured = (0..5)
        .map(|_| requests.recv_timeout(Duration::from_secs(1)).unwrap())
        .collect::<Vec<_>>();
    assert!(captured[0].starts_with("GET /clinics/clinic-id/diagnoses HTTP/1.1"));
    assert!(captured[1].starts_with(
        "GET /clinics/clinic-id/procedures/prices?sortBy=name&direction=ASC&currency=EUR HTTP/1.1"
    ));
    assert!(captured[2].starts_with(
        "GET /clinics/clinic-id/patients/patient-id/visits/visit-id/treatments HTTP/1.1"
    ));
    assert!(captured[3].starts_with(
        "PUT /clinics/clinic-id/patients/patient-id/visits/visit-id/treatments/treatment-row HTTP/1.1"
    ));
    assert!(captured[3].contains("\"note\":\"Бележка\""));
    assert!(captured[4].starts_with(
        "POST /clinics/clinic-id/patients/patient-id/treatments/treatment-row/procedures HTTP/1.1"
    ));
    assert!(captured[4].contains("\"procedureId\":\"procedure-id\""));
}

#[tokio::test]
async fn nzok_status_visit_uses_is_nzok_true_and_independent_active_visit_readback() {
    let visit =
        r#"{"id":"visit-id","createdStatusUpdate":false,"isFinished":false,"cancelled":false}"#;
    let verified =
        r#"{"id":"visit-id","createdStatusUpdate":true,"isFinished":false,"cancelled":false}"#;
    let (base, requests) = scripted_server(vec![
        ResponseScript::json(200, visit),
        ResponseScript::json(200, r#"{"teethStatus":[]}"#),
        ResponseScript::json(200, verified),
    ]);
    let client = AidooClient::for_test(base, Duration::from_secs(2));
    let result = create_status_visit(
        &client,
        "token",
        "clinic-id",
        "patient-id",
        "doctor-id",
        true,
    )
    .await
    .unwrap();
    assert!(result.is_nzok);
    assert_eq!(result.verification.outcome, VerificationOutcome::Verified);
    let captured = (0..3)
        .map(|_| requests.recv_timeout(Duration::from_secs(1)).unwrap())
        .collect::<Vec<_>>();
    assert!(captured[0].starts_with("POST /clinics/clinic-id/patients/patient-id/visits HTTP/1.1"));
    assert!(captured[0].contains("\"doctorId\":\"doctor-id\""));
    assert!(captured[1].starts_with(
        "POST /clinics/clinic-id/patients/patient-id/teeth-status?visitId=visit-id&isNzok=true HTTP/1.1"
    ));
    assert!(captured[2]
        .starts_with("GET /clinics/clinic-id/patients/patient-id/visits/active HTTP/1.1"));
}

#[tokio::test]
async fn treatment_workflow_reads_before_writes_and_verifies_every_field() {
    let before = treatment_json("treatment-row", None, None, &[]);
    let after_without_procedure = treatment_json(
        "treatment-row",
        Some("diagnosis-id"),
        Some("Официална забележка"),
        &[],
    );
    let after = treatment_json(
        "treatment-row",
        Some("diagnosis-id"),
        Some("Официална забележка"),
        &["procedure-id"],
    );
    let (base, requests) = scripted_server(vec![
        ResponseScript::json(200, &format!("[{before}]")),
        ResponseScript::json(200, &after_without_procedure),
        ResponseScript::json(
            200,
            r#"{"procedure":{"id":"joined-id","procedureId":"procedure-id","price":"42.5","discount":"0"},"treatmentId":"treatment-row"}"#,
        ),
        ResponseScript::json(200, &format!("[{after}]")),
    ]);
    let client = AidooClient::for_test(base, Duration::from_secs(2));
    let draft = build_treatment_draft(
        "patient-id".into(),
        &visit(true),
        vec![treatment("treatment-row", None, None, None, &[])],
        &diagnosis_catalog(),
        &procedure_catalog(),
        TreatmentChange {
            tooth: "26".into(),
            existing_treatment_id: Some("treatment-row".into()),
            diagnosis_id: Some("diagnosis-id".into()),
            treatment_id: None,
            note: Some("Официална забележка".into()),
            procedure_ids: vec!["procedure-id".into()],
        },
    )
    .unwrap();
    let result = apply_confirmed_treatment_draft(&client, "token", "clinic-id", &draft)
        .await
        .unwrap();
    assert_eq!(result.outcome, VerificationOutcome::Verified);
    let captured = (0..4)
        .map(|_| requests.recv_timeout(Duration::from_secs(1)).unwrap())
        .collect::<Vec<_>>();
    assert_eq!(
        captured
            .iter()
            .filter(|request| request.starts_with("PUT "))
            .count(),
        1
    );
    assert_eq!(
        captured
            .iter()
            .filter(|request| request.starts_with("POST "))
            .count(),
        1
    );
    assert!(captured.last().unwrap().starts_with("GET "));
}

#[tokio::test]
async fn stale_draft_is_rejected_before_post_or_put() {
    let changed = serde_json::to_string(&status("32", &["restoration-id"], &["OCCLUSAL"])).unwrap();
    let (base, requests) = scripted_server(vec![ResponseScript::json(
        200,
        &format!(r#"{{"teethStatus":[{changed}]}}"#),
    )]);
    let client = AidooClient::for_test(base, Duration::from_secs(2));
    let draft = build_draft(
        "patient-id".into(),
        &visit(false),
        false,
        vec![status("32", &[], &["OCCLUSAL"])],
        &catalog(),
        &[StatusChange {
            operation: StatusOperation::Add,
            tooth: "32".into(),
            status_id: "caries-id".into(),
            regions: vec!["OCCLUSAL".into()],
            existing_status_id: None,
            is_milk_tooth: false,
            for_observation: false,
            note: None,
        }],
    )
    .unwrap();

    let result = apply_confirmed_draft(&client, "token", "clinic-id", &draft)
        .await
        .unwrap();
    assert_eq!(result.outcome, VerificationOutcome::StaleDraft);
    assert!(requests.recv_timeout(Duration::from_secs(2)).is_ok());
    assert!(requests.recv_timeout(Duration::from_millis(150)).is_err());
}

#[tokio::test]
async fn workflow_reads_before_write_and_verifies_with_the_visit_endpoint() {
    let empty = status_json("32", &[], &[]);
    let desired = status_json("32", &["caries-id"], &["OCCLUSAL"]);
    let (base, requests) = scripted_server(vec![
        ResponseScript::json(200, &format!(r#"{{"teethStatus":[{empty}]}}"#)),
        ResponseScript::json(200, &format!(r#"{{"teethStatus":[{desired}]}}"#)),
        ResponseScript::json(
            200,
            &format!(
                r#"{{"visitTeethStatus":[{{"currentToothStatus":{desired},"previousToothStatus":null}}]}}"#
            ),
        ),
    ]);
    let client = AidooClient::for_test(base, Duration::from_secs(2));
    let draft = surface_draft();

    let result = apply_confirmed_draft(&client, "session-token", "clinic-id", &draft)
        .await
        .unwrap();

    assert_eq!(result.outcome, VerificationOutcome::Verified);
    let captured = (0..3)
        .map(|_| requests.recv_timeout(Duration::from_secs(1)).unwrap())
        .collect::<Vec<_>>();
    assert!(captured[0].starts_with("GET /clinics/clinic-id/patients/patient-id/teeth-status?"));
    assert!(captured[1].starts_with("PUT /clinics/clinic-id/patients/patient-id/teeth-status?"));
    assert!(captured[2].starts_with(
        "GET /clinics/clinic-id/patients/patient-id/teeth-status/visits/visit-id HTTP/1.1"
    ));
}

#[tokio::test]
async fn ambiguous_write_is_not_repeated_and_can_be_verified_by_read_back() {
    let empty = status_json("32", &[], &[]);
    let desired = status_json("32", &["caries-id"], &["OCCLUSAL"]);
    let (base, requests) = scripted_server(vec![
        ResponseScript::json(200, &format!(r#"{{"teethStatus":[{empty}]}}"#)),
        ResponseScript::delayed_json(
            200,
            &format!(r#"{{"teethStatus":[{desired}]}}"#),
            Duration::from_millis(250),
        ),
        ResponseScript::json(
            200,
            &format!(
                r#"{{"visitTeethStatus":[{{"currentToothStatus":{desired},"previousToothStatus":null}}]}}"#
            ),
        ),
    ]);
    let client = AidooClient::for_test(base, Duration::from_millis(80));

    let result = apply_confirmed_draft(&client, "session-token", "clinic-id", &surface_draft())
        .await
        .unwrap();

    assert_eq!(
        result.outcome,
        VerificationOutcome::VerifiedAfterAmbiguousWrite
    );
    let captured = (0..3)
        .map(|_| requests.recv_timeout(Duration::from_secs(1)).unwrap())
        .collect::<Vec<_>>();
    assert_eq!(
        captured
            .iter()
            .filter(|request| request.starts_with("PUT "))
            .count(),
        1
    );
}

fn surface_draft() -> StatusDraft {
    build_draft(
        "patient-id".into(),
        &visit(true),
        false,
        vec![status("32", &[], &[])],
        &catalog(),
        &[StatusChange {
            operation: StatusOperation::Add,
            tooth: "32".into(),
            status_id: "caries-id".into(),
            regions: vec!["OCCLUSAL".into()],
            existing_status_id: None,
            is_milk_tooth: false,
            for_observation: false,
            note: None,
        }],
    )
    .unwrap()
}

fn status_json(tooth: &str, statuses: &[&str], regions: &[&str]) -> String {
    serde_json::json!({
        "id": format!("record-{tooth}"),
        "tooth": tooth,
        "statuses": statuses,
        "isMilkTooth": false,
        "forObservation": false,
        "regions": regions,
        "timestamp": "2026-09-16T00:00:00Z",
        "note": null,
        "generatedByProcedure": false
    })
    .to_string()
}

fn diagnosis_catalog() -> Vec<DiagnosisCatalogEntry> {
    vec![DiagnosisCatalogEntry {
        id: "diagnosis-id".into(),
        name: "Кариес на дентина".into(),
        key: "K02.1".into(),
    }]
}

fn procedure_catalog() -> Vec<ProcedureCatalogEntry> {
    vec![ProcedureCatalogEntry {
        id: "procedure-id".into(),
        name: "Обтурация".into(),
        key: "P1".into(),
        price: serde_json::json!(42.5),
        price_currency: Some("EUR".into()),
    }]
}

fn treatment(
    id: &str,
    diagnosis_id: Option<&str>,
    note: Option<&str>,
    treatment_id: Option<&str>,
    procedure_ids: &[&str],
) -> VisitTreatment {
    VisitTreatment {
        id: id.into(),
        tooth: "26".into(),
        diagnosis_id: diagnosis_id.map(str::to_string),
        treatment_id: treatment_id.map(str::to_string),
        note: note.map(str::to_string),
        status: None,
        is_milk_tooth: false,
        procedures: procedure_ids
            .iter()
            .map(|id| TreatmentProcedure {
                id: Some(format!("joined-{id}")),
                procedure_id: (*id).into(),
                price: serde_json::json!("42.5"),
                discount: serde_json::json!("0"),
            })
            .collect(),
    }
}

fn treatment_json(
    id: &str,
    diagnosis_id: Option<&str>,
    note: Option<&str>,
    procedure_ids: &[&str],
) -> String {
    serde_json::to_string(&treatment(id, diagnosis_id, note, None, procedure_ids)).unwrap()
}

struct ResponseScript {
    status: u16,
    body: String,
    delay: Duration,
}

impl ResponseScript {
    fn json(status: u16, body: &str) -> Self {
        Self {
            status,
            body: body.into(),
            delay: Duration::ZERO,
        }
    }

    fn delayed_json(status: u16, body: &str, delay: Duration) -> Self {
        Self {
            status,
            body: body.into(),
            delay,
        }
    }
}

fn scripted_server(responses: Vec<ResponseScript>) -> (String, mpsc::Receiver<String>) {
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let address = listener.local_addr().unwrap();
    let responses = Arc::new(Mutex::new(responses.into_iter()));
    let (sender, receiver) = mpsc::channel();
    thread::spawn(move || {
        for stream in listener.incoming() {
            let Ok(stream) = stream else { break };
            let Some(script) = responses.lock().unwrap().next() else {
                break;
            };
            let sender = sender.clone();
            thread::spawn(move || serve(stream, script, sender));
        }
    });
    (format!("http://{address}"), receiver)
}

fn serve(mut stream: TcpStream, script: ResponseScript, sender: mpsc::Sender<String>) {
    stream.set_read_timeout(Some(Duration::from_secs(2))).ok();
    let mut request = Vec::new();
    let mut buffer = [0_u8; 4096];
    let mut expected = None;
    loop {
        let Ok(read) = stream.read(&mut buffer) else {
            return;
        };
        if read == 0 {
            break;
        }
        request.extend_from_slice(&buffer[..read]);
        if expected.is_none() {
            if let Some(header_end) = find_bytes(&request, b"\r\n\r\n") {
                let headers = String::from_utf8_lossy(&request[..header_end]);
                let length = headers
                    .lines()
                    .find_map(|line| {
                        line.to_ascii_lowercase()
                            .strip_prefix("content-length:")
                            .map(str::trim)
                            .and_then(|value| value.parse::<usize>().ok())
                    })
                    .unwrap_or(0);
                expected = Some(header_end + 4 + length);
            }
        }
        if expected.is_some_and(|length| request.len() >= length) {
            break;
        }
    }
    let _ = sender.send(String::from_utf8_lossy(&request).into_owned());
    thread::sleep(script.delay);
    let reason = if script.status == 200 { "OK" } else { "Error" };
    let response = format!(
        "HTTP/1.1 {} {}\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",
        script.status,
        reason,
        script.body.len(),
        script.body
    );
    let _ = stream.write_all(response.as_bytes());
}

fn find_bytes(haystack: &[u8], needle: &[u8]) -> Option<usize> {
    haystack
        .windows(needle.len())
        .position(|window| window == needle)
}
