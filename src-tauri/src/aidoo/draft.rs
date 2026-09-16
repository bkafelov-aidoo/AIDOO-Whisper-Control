use super::types::*;
use std::collections::{BTreeMap, BTreeSet};

pub fn build_draft(
    patient_id: String,
    visit: &Visit,
    is_nzok: bool,
    baseline: Vec<ToothStatus>,
    catalog: &[StatusCatalogEntry],
    changes: &[StatusChange],
) -> Result<StatusDraft, String> {
    if changes.is_empty() {
        return Err("Липсва промяна за зъбния статус.".into());
    }
    if patient_id.trim().is_empty() || visit.id.trim().is_empty() {
        return Err("Липсва пациент или посещение.".into());
    }
    let catalog_ids = catalog
        .iter()
        .map(|entry| entry.id.as_str())
        .collect::<BTreeSet<_>>();
    let mut state = status_map(&baseline);
    let mut touched = BTreeSet::new();

    for change in changes {
        validate_change(change, &catalog_ids)?;
        let regions = sorted_unique(&change.regions);
        let key = (change.tooth.clone(), regions.clone());
        let entry = state
            .entry(key.clone())
            .or_insert_with(|| ToothStatusWrite {
                tooth: change.tooth.clone(),
                statuses: Vec::new(),
                is_milk_tooth: change.is_milk_tooth,
                for_observation: change.for_observation,
                regions: regions.clone(),
                note: change.note.clone(),
            });
        if !regions.is_empty() {
            entry.is_milk_tooth = false;
            entry.for_observation = false;
        }
        if let Some(note) = &change.note {
            entry.note = Some(note.clone());
        }
        match change.operation {
            StatusOperation::Add => entry.statuses.push(change.status_id.clone()),
            StatusOperation::Replace => {
                let existing = change
                    .existing_status_id
                    .as_ref()
                    .ok_or_else(|| "При замяна трябва да е посочен текущият статус.".to_string())?;
                if !entry.statuses.iter().any(|status| status == existing) {
                    return Err("Статусът за замяна вече не присъства в текущия запис.".into());
                }
                entry.statuses.retain(|status| status != existing);
                entry.statuses.push(change.status_id.clone());
            }
        }
        entry.statuses = sorted_unique(&entry.statuses);
        touched.insert(key);
    }

    let writes = touched
        .into_iter()
        .filter_map(|key| state.remove(&key))
        .collect::<Vec<_>>();
    Ok(StatusDraft {
        id: uuid::Uuid::new_v4().to_string(),
        patient_id,
        visit_id: visit.id.clone(),
        is_nzok,
        create_status_update: !visit.created_status_update,
        baseline,
        spoken_summary: spoken_summary(changes, catalog)?,
        writes,
    })
}

pub fn same_snapshot(left: &[ToothStatus], right: &[ToothStatus]) -> bool {
    normalized_snapshot(left) == normalized_snapshot(right)
}

pub fn verifies(writes: &[ToothStatusWrite], actual: &[ToothStatus]) -> bool {
    let actual = status_map(actual);
    writes.iter().all(|expected| {
        let key = (expected.tooth.clone(), sorted_unique(&expected.regions));
        actual.get(&key).is_some_and(|found| {
            found.statuses == sorted_unique(&expected.statuses)
                && found.is_milk_tooth == expected.is_milk_tooth
                && found.for_observation == expected.for_observation
                && found.note == expected.note
        })
    })
}

fn normalized_snapshot(
    values: &[ToothStatus],
) -> BTreeMap<(String, Vec<String>), ToothStatusWrite> {
    status_map(values)
}

fn validate_change(change: &StatusChange, catalog_ids: &BTreeSet<&str>) -> Result<(), String> {
    if !valid_tooth(&change.tooth) {
        return Err("Невалиден номер на зъб.".into());
    }
    if !catalog_ids.contains(change.status_id.as_str()) {
        return Err("Избраният статус липсва в актуалния AIDOO каталог.".into());
    }
    if let Some(existing) = &change.existing_status_id {
        if !catalog_ids.contains(existing.as_str()) {
            return Err("Статусът за замяна липсва в актуалния AIDOO каталог.".into());
        }
    }
    if change
        .regions
        .iter()
        .any(|region| !ALLOWED_REGIONS.contains(&region.as_str()))
    {
        return Err("Невалидна зъбна повърхност.".into());
    }
    if change.regions.is_empty() && change.is_milk_tooth && !is_milk_tooth(&change.tooth) {
        return Err("Постоянен зъб не може да бъде маркиран като временен.".into());
    }
    Ok(())
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

fn spoken_summary(
    changes: &[StatusChange],
    catalog: &[StatusCatalogEntry],
) -> Result<String, String> {
    let names = catalog
        .iter()
        .map(|entry| (entry.id.as_str(), entry.name.as_str()))
        .collect::<BTreeMap<_, _>>();
    let mut parts = Vec::with_capacity(changes.len());
    for change in changes {
        let name = names
            .get(change.status_id.as_str())
            .ok_or_else(|| "Липсва име за избрания статус.".to_string())?;
        let surface = if change.regions.is_empty() {
            String::new()
        } else {
            format!(" ({})", change.regions.join(", "))
        };
        let verb = match change.operation {
            StatusOperation::Add => "добавя",
            StatusOperation::Replace => "заменя със",
        };
        parts.push(format!("{verb} {name}{surface} на зъб {}", change.tooth));
    }
    Ok(format!("Ще {}. Да го запиша ли?", parts.join(" и ")))
}
