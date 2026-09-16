use super::client::{AidooClient, AidooError};
use super::draft::{same_snapshot, verifies};
use super::treatment::{same_treatment_snapshot, verifies_treatment};
use super::types::*;

pub async fn apply_confirmed_draft(
    client: &AidooClient,
    session: &str,
    clinic_id: &str,
    draft: &StatusDraft,
) -> Result<VerificationResult, AidooError> {
    let current = client
        .editable_status(
            session,
            clinic_id,
            &draft.patient_id,
            &draft.visit_id,
            draft.is_nzok,
        )
        .await?;
    if !same_snapshot(&draft.baseline, &current.teeth_status) {
        return Ok(VerificationResult {
            outcome: VerificationOutcome::StaleDraft,
            message: "Статусът е променен след подготовката. Прочетете го отново и потвърдете нова чернова.".into(),
        });
    }

    if draft.create_status_update {
        client
            .create_status_update(
                session,
                clinic_id,
                &draft.patient_id,
                &draft.visit_id,
                draft.is_nzok,
            )
            .await?;
    }

    match client
        .write_status(
            session,
            clinic_id,
            &draft.patient_id,
            &draft.visit_id,
            &draft.writes,
        )
        .await
    {
        Ok(_) => verify_independently(client, session, clinic_id, draft, false).await,
        Err(error) if error.is_ambiguous_write() => {
            match verify_independently(client, session, clinic_id, draft, true).await {
                Ok(result) => Ok(result),
                Err(_) => Ok(VerificationResult {
                    outcome: VerificationOutcome::Uncertain,
                    message:
                        "Записът не можа да бъде потвърден. Не повтаряйте действието автоматично."
                            .into(),
                }),
            }
        }
        Err(error) => Ok(VerificationResult {
            outcome: VerificationOutcome::Rejected,
            message: error.message,
        }),
    }
}

pub async fn create_status_visit(
    client: &AidooClient,
    session: &str,
    clinic_id: &str,
    patient_id: &str,
    doctor_id: &str,
    is_nzok: bool,
) -> Result<StatusVisitResult, AidooError> {
    let visit = client
        .create_visit(session, clinic_id, patient_id, doctor_id)
        .await?;
    let creation = client
        .create_status_update(session, clinic_id, patient_id, &visit.id, is_nzok)
        .await;
    let active = client.active_visit(session, clinic_id, patient_id).await;
    let verified = active
        .as_ref()
        .is_ok_and(|active| active.id == visit.id && active.created_status_update);
    let verification = match creation {
        Ok(_) if verified => VerificationResult {
            outcome: VerificationOutcome::Verified,
            message: format!(
                "Създадено е {} посещение за статус и записът е потвърден.",
                if is_nzok { "НЗОК" } else { "частно" }
            ),
        },
        Err(error) if error.is_ambiguous_write() && verified => VerificationResult {
            outcome: VerificationOutcome::VerifiedAfterAmbiguousWrite,
            message: "AIDOO прекъсна отговора, но посещението за статус е потвърдено с независимо прочитане.".into(),
        },
        Err(error) if !error.is_ambiguous_write() => VerificationResult {
            outcome: VerificationOutcome::Rejected,
            message: format!(
                "Посещението е създадено, но статусният запис не беше създаден: {}",
                error.message
            ),
        },
        _ => VerificationResult {
            outcome: VerificationOutcome::Uncertain,
            message: "Посещението е създадено, но статусният запис не можа да бъде потвърден. Не повтаряйте автоматично.".into(),
        },
    };
    Ok(StatusVisitResult {
        visit,
        is_nzok,
        verification,
    })
}

pub async fn apply_confirmed_treatment_draft(
    client: &AidooClient,
    session: &str,
    clinic_id: &str,
    draft: &TreatmentDraft,
) -> Result<VerificationResult, AidooError> {
    let current = client
        .visit_treatments(session, clinic_id, &draft.patient_id, &draft.visit_id)
        .await?;
    if !same_treatment_snapshot(&draft.baseline, &current) {
        return Ok(VerificationResult {
            outcome: VerificationOutcome::StaleDraft,
            message: "Леченията са променени след подготовката. Прочетете ги отново и потвърдете нова чернова.".into(),
        });
    }

    let write = match &draft.existing_treatment_id {
        Some(id) => {
            client
                .update_treatment(
                    session,
                    clinic_id,
                    &draft.patient_id,
                    &draft.visit_id,
                    id,
                    &draft.treatment,
                )
                .await
        }
        None => {
            client
                .create_treatment(
                    session,
                    clinic_id,
                    &draft.patient_id,
                    &draft.visit_id,
                    &draft.treatment,
                )
                .await
        }
    };
    let treatment_id = match write {
        Ok(treatment) => treatment.id,
        Err(error) if error.is_ambiguous_write() => {
            return verify_treatment(client, session, clinic_id, draft, true).await;
        }
        Err(error) => {
            return Ok(VerificationResult {
                outcome: VerificationOutcome::Rejected,
                message: error.message,
            });
        }
    };

    for procedure in &draft.procedures {
        if let Err(error) = client
            .add_procedure(
                session,
                clinic_id,
                &draft.patient_id,
                &treatment_id,
                procedure,
            )
            .await
        {
            if error.is_ambiguous_write() {
                return verify_treatment(client, session, clinic_id, draft, true).await;
            }
            return Ok(VerificationResult {
                outcome: VerificationOutcome::Uncertain,
                message: format!(
                    "Редът за лечение е записан, но не всички процедури са добавени: {}. Проверете записа преди повторение.",
                    error.message
                ),
            });
        }
    }
    verify_treatment(client, session, clinic_id, draft, false).await
}

async fn verify_treatment(
    client: &AidooClient,
    session: &str,
    clinic_id: &str,
    draft: &TreatmentDraft,
    after_ambiguous_write: bool,
) -> Result<VerificationResult, AidooError> {
    let actual = client
        .visit_treatments(session, clinic_id, &draft.patient_id, &draft.visit_id)
        .await?;
    if verifies_treatment(draft, &actual) {
        Ok(VerificationResult {
            outcome: if after_ambiguous_write {
                VerificationOutcome::VerifiedAfterAmbiguousWrite
            } else {
                VerificationOutcome::Verified
            },
            message: "Диагнозата, процедурите и официалната забележка са записани и потвърдени с независимо прочитане.".into(),
        })
    } else {
        Ok(VerificationResult {
            outcome: VerificationOutcome::Uncertain,
            message: "AIDOO прие част от заявката, но независимото прочитане не потвърди целия очакван запис. Не повтаряйте автоматично.".into(),
        })
    }
}

async fn verify_independently(
    client: &AidooClient,
    session: &str,
    clinic_id: &str,
    draft: &StatusDraft,
    after_ambiguous_write: bool,
) -> Result<VerificationResult, AidooError> {
    let read_back = client
        .visit_status(session, clinic_id, &draft.patient_id, &draft.visit_id)
        .await?;
    let current = read_back
        .visit_teeth_status
        .into_iter()
        .map(|entry| entry.current_tooth_status)
        .collect::<Vec<_>>();
    if verifies(&draft.writes, &current) {
        Ok(VerificationResult {
            outcome: if after_ambiguous_write {
                VerificationOutcome::VerifiedAfterAmbiguousWrite
            } else {
                VerificationOutcome::Verified
            },
            message: "Зъбният статус е записан и потвърден с независимо прочитане.".into(),
        })
    } else {
        Ok(VerificationResult {
            outcome: VerificationOutcome::Uncertain,
            message: "AIDOO прие заявката, но независимото прочитане не потвърди очаквания статус."
                .into(),
        })
    }
}
