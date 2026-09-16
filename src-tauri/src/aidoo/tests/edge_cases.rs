use super::*;

#[test]
fn treatment_draft_targets_one_of_multiple_rows_and_allows_note_only() {
    let baseline = vec![
        treatment("row-a", None, Some("Първи ред"), None, &[]),
        treatment("row-b", None, Some("Втори ред"), None, &[]),
    ];
    let draft = build_treatment_draft(
        "patient-id".into(),
        &visit(true),
        baseline,
        &[],
        &[],
        TreatmentChange {
            tooth: "26".into(),
            existing_treatment_id: Some("row-b".into()),
            diagnosis_id: None,
            treatment_id: None,
            note: Some("  Само официална забележка.  ".into()),
            procedure_ids: vec![],
        },
    )
    .unwrap();

    assert_eq!(draft.existing_treatment_id.as_deref(), Some("row-b"));
    assert_eq!(
        draft.treatment.note.as_deref(),
        Some("Само официална забележка.")
    );
    assert!(draft.procedures.is_empty());
}

#[test]
fn treatment_draft_rejects_existing_procedure_and_unverified_treatment_pair() {
    let duplicate = build_treatment_draft(
        "patient-id".into(),
        &visit(true),
        vec![treatment(
            "treatment-row",
            None,
            None,
            None,
            &["procedure-id"],
        )],
        &[],
        &procedure_catalog(),
        TreatmentChange {
            tooth: "26".into(),
            existing_treatment_id: Some("treatment-row".into()),
            diagnosis_id: None,
            treatment_id: None,
            note: None,
            procedure_ids: vec!["procedure-id".into()],
        },
    )
    .unwrap_err();
    assert!(duplicate.contains("вече съществува"));

    let incompatible = build_treatment_draft(
        "patient-id".into(),
        &visit(true),
        vec![treatment(
            "treatment-row",
            Some("old-diagnosis"),
            None,
            Some("linked-treatment"),
            &[],
        )],
        &diagnosis_catalog(),
        &[],
        TreatmentChange {
            tooth: "26".into(),
            existing_treatment_id: Some("treatment-row".into()),
            diagnosis_id: Some("diagnosis-id".into()),
            treatment_id: Some("linked-treatment".into()),
            note: None,
            procedure_ids: vec![],
        },
    )
    .unwrap_err();
    assert!(incompatible.contains("свързано лечение"));
}

#[test]
fn treatment_draft_uses_the_catalog_price_for_the_clinic_currency() {
    let procedures = vec![ProcedureCatalogEntry {
        id: "procedure-bgn".into(),
        name: "Процедура в лева".into(),
        key: "BG1".into(),
        price: serde_json::json!("95.00"),
        price_currency: Some("BGN".into()),
    }];
    let draft = build_treatment_draft(
        "patient-id".into(),
        &visit(true),
        vec![treatment("treatment-row", None, None, None, &[])],
        &[],
        &procedures,
        TreatmentChange {
            tooth: "26".into(),
            existing_treatment_id: Some("treatment-row".into()),
            diagnosis_id: None,
            treatment_id: None,
            note: None,
            procedure_ids: vec!["procedure-bgn".into()],
        },
    )
    .unwrap();

    assert_eq!(draft.procedures[0].price, "95.00");
}

#[test]
fn treatment_snapshot_ignores_order_but_detects_row_changes() {
    let first = treatment("row-a", None, None, None, &["procedure-a", "procedure-b"]);
    let second = treatment("row-b", None, Some("Бележка"), None, &[]);
    let mut reordered_first = first.clone();
    reordered_first.procedures.reverse();
    assert!(same_treatment_snapshot(
        &[first.clone(), second.clone()],
        &[second.clone(), reordered_first]
    ));

    let unchanged_first = first.clone();
    let mut changed = second;
    changed.note = Some("Променено от браузъра".into());
    assert!(!same_treatment_snapshot(
        &[first, treatment("row-b", None, Some("Бележка"), None, &[])],
        &[unchanged_first, changed]
    ));
}

#[tokio::test]
async fn procedure_catalog_uses_the_clinic_currency_without_rewriting_prices() {
    let (base, requests) = scripted_server(vec![ResponseScript::json(
        200,
        r#"[{"id":"procedure-bgn","name":"Процедура","key":"BG1","price":"95.00","priceCurrency":"BGN"}]"#,
    )]);
    let client = AidooClient::for_test(base, Duration::from_secs(2));
    let catalog = client
        .procedure_catalog("token", "clinic-id", Some("BGN"))
        .await
        .unwrap();

    assert_eq!(catalog[0].price_text(), "95.00");
    assert_eq!(catalog[0].price_currency.as_deref(), Some("BGN"));
    let request = requests.recv_timeout(Duration::from_secs(1)).unwrap();
    assert!(request.starts_with(
        "GET /clinics/clinic-id/procedures/prices?sortBy=name&direction=ASC&currency=BGN HTTP/1.1"
    ));
}

#[tokio::test]
async fn treatment_workflow_rejects_a_row_changed_in_the_browser_before_writing() {
    let before = treatment("treatment-row", None, Some("Преди"), None, &[]);
    let changed = treatment_json("treatment-row", None, Some("Променено в браузъра"), &[]);
    let (base, requests) =
        scripted_server(vec![ResponseScript::json(200, &format!("[{changed}]"))]);
    let client = AidooClient::for_test(base, Duration::from_secs(2));
    let draft = build_treatment_draft(
        "patient-id".into(),
        &visit(true),
        vec![before],
        &[],
        &[],
        TreatmentChange {
            tooth: "26".into(),
            existing_treatment_id: Some("treatment-row".into()),
            diagnosis_id: None,
            treatment_id: None,
            note: Some("Нова забележка".into()),
            procedure_ids: vec![],
        },
    )
    .unwrap();

    let result = apply_confirmed_treatment_draft(&client, "token", "clinic-id", &draft)
        .await
        .unwrap();
    assert_eq!(result.outcome, VerificationOutcome::StaleDraft);
    let request = requests.recv_timeout(Duration::from_secs(1)).unwrap();
    assert!(request.starts_with("GET "));
    assert!(requests.recv_timeout(Duration::from_millis(150)).is_err());
}

#[tokio::test]
async fn incompatible_first_procedure_is_rejected_without_an_earlier_write() {
    let before = treatment_json("treatment-row", None, None, &[]);
    let (base, requests) = scripted_server(vec![
        ResponseScript::json(200, &format!("[{before}]")),
        ResponseScript::json(422, r#"{"error":"incompatible procedure"}"#),
    ]);
    let client = AidooClient::for_test(base, Duration::from_secs(2));
    let draft = build_treatment_draft(
        "patient-id".into(),
        &visit(true),
        vec![treatment("treatment-row", None, None, None, &[])],
        &[],
        &procedure_catalog(),
        TreatmentChange {
            tooth: "26".into(),
            existing_treatment_id: Some("treatment-row".into()),
            diagnosis_id: None,
            treatment_id: None,
            note: None,
            procedure_ids: vec!["procedure-id".into()],
        },
    )
    .unwrap();

    let result = apply_confirmed_treatment_draft(&client, "token", "clinic-id", &draft)
        .await
        .unwrap();
    assert_eq!(result.outcome, VerificationOutcome::Rejected);
    let captured = (0..2)
        .map(|_| requests.recv_timeout(Duration::from_secs(1)).unwrap())
        .collect::<Vec<_>>();
    assert!(captured[0].starts_with("GET "));
    assert!(captured[1].starts_with("POST "));
    assert!(captured.iter().all(|request| !request.starts_with("PUT ")));
}

#[tokio::test]
async fn partial_procedure_success_is_read_back_and_never_retried() {
    let before = treatment_json("treatment-row", None, None, &[]);
    let after_first = treatment_json("treatment-row", None, None, &["procedure-a"]);
    let procedures = vec![
        ProcedureCatalogEntry {
            id: "procedure-a".into(),
            name: "Първа процедура".into(),
            key: "P1".into(),
            price: serde_json::json!(10),
            price_currency: Some("BGN".into()),
        },
        ProcedureCatalogEntry {
            id: "procedure-b".into(),
            name: "Несъвместима процедура".into(),
            key: "P2".into(),
            price: serde_json::json!(20),
            price_currency: Some("BGN".into()),
        },
    ];
    let (base, requests) = scripted_server(vec![
        ResponseScript::json(200, &format!("[{before}]")),
        ResponseScript::json(
            200,
            r#"{"procedure":{"id":"joined-a","procedureId":"procedure-a","price":"10","discount":"0"},"treatmentId":"treatment-row"}"#,
        ),
        ResponseScript::json(422, r#"{"error":"incompatible procedures"}"#),
        ResponseScript::json(200, &format!("[{after_first}]")),
    ]);
    let client = AidooClient::for_test(base, Duration::from_secs(2));
    let draft = build_treatment_draft(
        "patient-id".into(),
        &visit(true),
        vec![treatment("treatment-row", None, None, None, &[])],
        &[],
        &procedures,
        TreatmentChange {
            tooth: "26".into(),
            existing_treatment_id: Some("treatment-row".into()),
            diagnosis_id: None,
            treatment_id: None,
            note: None,
            procedure_ids: vec!["procedure-a".into(), "procedure-b".into()],
        },
    )
    .unwrap();

    let result = apply_confirmed_treatment_draft(&client, "token", "clinic-id", &draft)
        .await
        .unwrap();
    assert_eq!(result.outcome, VerificationOutcome::Uncertain);
    assert!(result.message.contains("Част от промените"));
    assert!(result.message.contains("Не повтаряйте автоматично"));
    let captured = (0..4)
        .map(|_| requests.recv_timeout(Duration::from_secs(1)).unwrap())
        .collect::<Vec<_>>();
    assert_eq!(
        captured
            .iter()
            .filter(|request| request.starts_with("POST "))
            .count(),
        2
    );
    assert!(captured.last().unwrap().starts_with("GET "));
}
