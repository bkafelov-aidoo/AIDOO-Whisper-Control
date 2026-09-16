use super::types::*;
use std::collections::{BTreeMap, BTreeSet};

pub fn build_treatment_draft(
    patient_id: String,
    visit: &Visit,
    baseline: Vec<VisitTreatment>,
    diagnoses: &[DiagnosisCatalogEntry],
    procedures: &[ProcedureCatalogEntry],
    change: TreatmentChange,
) -> Result<TreatmentDraft, String> {
    if patient_id.trim().is_empty() || visit.id.trim().is_empty() {
        return Err("Липсва пациент или посещение.".into());
    }
    if !valid_tooth(&change.tooth) {
        return Err("Невалиден номер на зъб.".into());
    }
    if change
        .note
        .as_ref()
        .is_some_and(|note| note.chars().count() > 10_000)
    {
        return Err("Официалната забележка е прекалено дълга.".into());
    }
    let diagnosis_names = diagnoses
        .iter()
        .map(|entry| (entry.id.as_str(), entry.name.as_str()))
        .collect::<BTreeMap<_, _>>();
    if change
        .diagnosis_id
        .as_ref()
        .is_some_and(|id| !diagnosis_names.contains_key(id.as_str()))
    {
        return Err("Избраната диагноза липсва в актуалния AIDOO каталог.".into());
    }
    let procedure_by_id = procedures
        .iter()
        .map(|entry| (entry.id.as_str(), entry))
        .collect::<BTreeMap<_, _>>();
    let requested_ids = change
        .procedure_ids
        .iter()
        .map(String::as_str)
        .collect::<BTreeSet<_>>();
    if requested_ids.len() != change.procedure_ids.len()
        || requested_ids
            .iter()
            .any(|id| !procedure_by_id.contains_key(id))
    {
        return Err("Избрана процедура липсва или се повтаря в актуалния AIDOO каталог.".into());
    }

    let existing = match &change.existing_treatment_id {
        Some(id) => Some(
            baseline
                .iter()
                .find(|entry| &entry.id == id)
                .ok_or_else(|| "Избраният ред за лечение вече не съществува.".to_string())?,
        ),
        None => None,
    };
    if existing.is_some_and(|entry| entry.tooth != change.tooth) {
        return Err("Избраният ред за лечение е за друг зъб.".into());
    }
    if change
        .note
        .as_ref()
        .is_some_and(|note| note.trim().is_empty())
    {
        return Err("Официалната забележка е празна.".into());
    }
    if let Some(requested_treatment_id) = change.treatment_id.as_deref() {
        let existing_treatment_id = existing.and_then(|entry| entry.treatment_id.as_deref());
        if existing_treatment_id != Some(requested_treatment_id) {
            return Err(
                "Избраното лечение не може да бъде проверено спрямо актуален AIDOO каталог. Изберете съществуващ ред или уточнете процедурата."
                    .into(),
            );
        }
    }
    if change.diagnosis_id.as_ref().is_some_and(|diagnosis_id| {
        existing.is_some_and(|entry| {
            entry.treatment_id.is_some() && entry.diagnosis_id.as_ref() != Some(diagnosis_id)
        })
    }) {
        return Err(
            "Диагнозата не може да бъде сменена автоматично, защото редът има свързано лечение. Проверете съвместимостта в AIDOO."
                .into(),
        );
    }
    if change.diagnosis_id.is_none() && change.note.is_none() && change.procedure_ids.is_empty() {
        return Err("Липсват диагноза, процедура или официална забележка за запис.".into());
    }

    let treatment = TreatmentWrite {
        tooth: change.tooth.clone(),
        diagnosis_id: change
            .diagnosis_id
            .clone()
            .or_else(|| existing.and_then(|entry| entry.diagnosis_id.clone())),
        treatment_id: change
            .treatment_id
            .clone()
            .or_else(|| existing.and_then(|entry| entry.treatment_id.clone())),
        note: change
            .note
            .as_ref()
            .map(|note| note.trim().to_string())
            .or_else(|| existing.and_then(|entry| entry.note.clone())),
        status: existing.and_then(|entry| entry.status.clone()),
        is_milk_tooth: existing
            .map(|entry| entry.is_milk_tooth)
            .unwrap_or_else(|| is_milk_tooth(&change.tooth)),
    };
    let already_present = existing
        .map(|entry| {
            entry
                .procedures
                .iter()
                .map(|procedure| procedure.procedure_id.as_str())
                .collect::<BTreeSet<_>>()
        })
        .unwrap_or_default();
    let procedure_writes = change
        .procedure_ids
        .iter()
        .filter(|id| !already_present.contains(id.as_str()))
        .map(|id| {
            let entry = procedure_by_id[id.as_str()];
            ProcedureWrite {
                id: None,
                procedure_id: id.clone(),
                price: entry.price_text(),
                discount: "0".into(),
            }
        })
        .collect::<Vec<_>>();

    let changes_treatment = existing.is_none_or(|entry| {
        entry.diagnosis_id != treatment.diagnosis_id
            || entry.treatment_id != treatment.treatment_id
            || entry.note != treatment.note
            || entry.status != treatment.status
            || entry.is_milk_tooth != treatment.is_milk_tooth
    });
    if !changes_treatment && procedure_writes.is_empty() {
        return Err(if change.procedure_ids.is_empty() {
            "Няма промяна за запис в избрания ред за лечение.".into()
        } else {
            "Избраната процедура вече съществува в реда за лечение.".into()
        });
    }

    let mut summary = vec![format!("реда за зъб {}", change.tooth)];
    if let Some(id) = &change.diagnosis_id {
        summary.push(format!("диагноза {}", diagnosis_names[id.as_str()]));
    }
    if !procedure_writes.is_empty() {
        let names = procedure_writes
            .iter()
            .map(|write| procedure_by_id[write.procedure_id.as_str()].name.as_str())
            .collect::<Vec<_>>()
            .join(", ");
        summary.push(format!("процедури {names}"));
    }
    if change.note.is_some() {
        summary.push("официална забележка".into());
    }

    Ok(TreatmentDraft {
        id: uuid::Uuid::new_v4().to_string(),
        patient_id,
        visit_id: visit.id.clone(),
        existing_treatment_id: change.existing_treatment_id,
        baseline,
        treatment,
        procedures: procedure_writes,
        spoken_summary: format!("Ще запиша {}. Да го запиша ли?", summary.join(", ")),
    })
}

pub fn same_treatment_snapshot(left: &[VisitTreatment], right: &[VisitTreatment]) -> bool {
    normalized(left) == normalized(right)
}

pub fn verifies_treatment(draft: &TreatmentDraft, actual: &[VisitTreatment]) -> bool {
    let expected_id = draft.existing_treatment_id.as_deref();
    let baseline_ids = draft
        .baseline
        .iter()
        .map(|entry| entry.id.as_str())
        .collect::<BTreeSet<_>>();
    actual.iter().any(|entry| {
        expected_id
            .map(|id| entry.id == id)
            .unwrap_or_else(|| !baseline_ids.contains(entry.id.as_str()))
            && entry.tooth == draft.treatment.tooth
            && entry.diagnosis_id == draft.treatment.diagnosis_id
            && entry.treatment_id == draft.treatment.treatment_id
            && entry.note == draft.treatment.note
            && draft.procedures.iter().all(|expected| {
                entry
                    .procedures
                    .iter()
                    .any(|actual| actual.procedure_id == expected.procedure_id)
            })
    })
}

fn normalized(values: &[VisitTreatment]) -> Vec<VisitTreatment> {
    let mut values = values.to_vec();
    for value in &mut values {
        value
            .procedures
            .sort_by(|a, b| (&a.procedure_id, &a.id).cmp(&(&b.procedure_id, &b.id)));
    }
    values.sort_by(|a, b| a.id.cmp(&b.id));
    values
}

fn valid_tooth(value: &str) -> bool {
    if value == "*" {
        return true;
    }
    let Ok(number) = value.parse::<u8>() else {
        return false;
    };
    let quadrant = number / 10;
    let position = number % 10;
    matches!(quadrant, 1..=4) && matches!(position, 1..=8)
        || matches!(quadrant, 5..=8) && matches!(position, 1..=5)
}

fn is_milk_tooth(value: &str) -> bool {
    value
        .parse::<u8>()
        .is_ok_and(|number| matches!(number / 10, 5..=8))
}
