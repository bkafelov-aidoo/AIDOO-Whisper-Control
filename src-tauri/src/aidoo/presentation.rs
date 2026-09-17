use super::clinic::parse_clinic_reference;
use std::process::{Command, Stdio};
use std::time::{SystemTime, UNIX_EPOCH};
use tauri::{AppHandle, Emitter};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PatientView {
    Status,
    Treatment,
}

impl PatientView {
    fn mode(self) -> &'static str {
        match self {
            Self::Status => "status",
            Self::Treatment => "treatment",
        }
    }
}

const CHROME_PRESENT_SCRIPT: &str = r#"
on run argv
  set clinicPrefix to item 1 of argv
  set targetURL to item 2 of argv
  set navigationJavaScript to item 3 of argv
  set foundClinicTab to false
  tell application "Google Chrome"
    if (count of windows) is 0 then
      make new window
    end if
    repeat with browserWindow in windows
      repeat with tabIndex from 1 to (count of tabs of browserWindow)
        set browserTab to tab tabIndex of browserWindow
        set tabURL to URL of browserTab
        if tabURL starts with clinicPrefix then
          set foundClinicTab to true
          try
            set navigationResult to execute browserTab javascript navigationJavaScript
          on error
            return "javascript-disabled"
          end try
          if navigationResult is "navigated" then
            set active tab index of browserWindow to tabIndex
            set index of browserWindow to 1
            activate
            return "reused-spa"
          end if
        end if
      end repeat
    end repeat
    if foundClinicTab then
      return "no-ready-tab"
    end if
    tell front window
      make new tab at end of tabs with properties {URL:targetURL}
      set active tab index to (count of tabs)
    end tell
    activate
    return "opened"
  end tell
end run
"#;

pub fn present_patient(app: AppHandle, clinic_link: String, patient_id: String, view: PatientView) {
    let target = match patient_view_url(&clinic_link, &patient_id, view, sync_nonce()) {
        Ok(target) => target,
        Err(error) => {
            let _ = app.emit("toast", error);
            return;
        }
    };
    present_target(
        app,
        target,
        "AIDOO промяната е запазена, но пациентският екран не можа да бъде показан.",
    );
}

pub fn present_schedule(app: AppHandle, clinic_link: String, date: String, doctor_id: String) {
    let target = match schedule_view_url(&clinic_link, &date, &doctor_id, sync_nonce()) {
        Ok(target) => target,
        Err(error) => {
            let _ = app.emit("toast", error);
            return;
        }
    };
    present_target(
        app,
        target,
        "Графикът е обработен, но страницата му не можа да бъде показана.",
    );
}

fn present_target(app: AppHandle, target: PatientViewTarget, failure_message: &'static str) {
    let thread_app = app.clone();
    let _ = std::thread::Builder::new()
        .name("aidoo-browser-presentation".into())
        .spawn(move || {
            match present_in_chrome(&target.clinic_prefix, &target.url) {
                Ok(()) => return,
                Err(ChromePresentationError::JavascriptDisabled) => {
                    crate::storage::append_diagnostic(
                        "AIDOO browser presentation requires JavaScript from Apple Events",
                    );
                    let _ = thread_app.emit(
                        "toast",
                        "Chrome блокира показването на AIDOO. В Chrome включете View → Developer → Allow JavaScript from Apple Events и опитайте отново.",
                    );
                    return;
                }
                Err(ChromePresentationError::NoReadyTab) => {
                    crate::storage::append_diagnostic(
                        "AIDOO browser presentation found only an unloaded clinic tab",
                    );
                    let _ = thread_app.emit(
                        "toast",
                        "AIDOO табът е празен заради остарял browser cache. Отворете работещ AIDOO екран в Chrome и опитайте отново.",
                    );
                    return;
                }
                Err(ChromePresentationError::Unavailable) => {}
            }
            if open_in_chrome(&target.url).is_ok() {
                return;
            }
            if open_in_default_browser(&target.url).is_ok() {
                return;
            }
            crate::storage::append_diagnostic("AIDOO browser presentation failed");
            let _ = thread_app.emit("toast", failure_message);
        });
}

struct PatientViewTarget {
    clinic_prefix: String,
    url: String,
}

fn patient_view_url(
    clinic_link: &str,
    patient_id: &str,
    view: PatientView,
    nonce: u128,
) -> Result<PatientViewTarget, String> {
    if patient_id.is_empty()
        || patient_id.len() > 128
        || !patient_id
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'_'))
    {
        return Err(
            "Пациентският екран не може да бъде показан за невалиден идентификатор.".into(),
        );
    }
    let clinic = parse_clinic_reference(clinic_link)?;
    let record_base = clinic.url.strip_suffix("/login").ok_or_else(|| {
        "Линкът към AIDOO клиниката не съдържа валиден входен маршрут.".to_string()
    })?;
    Ok(PatientViewTarget {
        clinic_prefix: format!("{record_base}/"),
        url: format!(
            "{record_base}/medical-record?patientid={patient_id}&tab=record&mode={}&selectedTeeth=&triggerNzokChecksProp=true&aidooControlSync={nonce}",
            view.mode()
        ),
    })
}

fn schedule_view_url(
    clinic_link: &str,
    date: &str,
    doctor_id: &str,
    nonce: u128,
) -> Result<PatientViewTarget, String> {
    if chrono::NaiveDate::parse_from_str(date, "%Y-%m-%d").is_err()
        || doctor_id.is_empty()
        || doctor_id.len() > 128
        || !doctor_id
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'_'))
    {
        return Err("Графикът не може да бъде показан за невалидна дата или лекар.".into());
    }
    let clinic = parse_clinic_reference(clinic_link)?;
    let record_base = clinic.url.strip_suffix("/login").ok_or_else(|| {
        "Линкът към AIDOO клиниката не съдържа валиден входен маршрут.".to_string()
    })?;
    Ok(PatientViewTarget {
        clinic_prefix: format!("{record_base}/"),
        url: format!(
            "{record_base}/schedule?mode=doctors&active-date={date}&selected-doctors=%5B%22{doctor_id}%22%5D&aidooControlSync={nonce}"
        ),
    })
}

fn sync_nonce() -> u128 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|duration| duration.as_millis())
        .unwrap_or(0)
}

fn spa_navigation_script(url: &str) -> Result<String, String> {
    let encoded_url = serde_json::to_string(url)
        .map_err(|_| "The AIDOO browser route could not be encoded".to_string())?;
    Ok(format!(
        "(()=>{{if(!document.body||!document.body.innerText.trim())return'not-ready';window.history.pushState({{aidooControl:true}},'',{encoded_url});window.dispatchEvent(new PopStateEvent('popstate'));return'navigated';}})()"
    ))
}

#[derive(Debug, PartialEq, Eq)]
enum ChromePresentationError {
    JavascriptDisabled,
    NoReadyTab,
    Unavailable,
}

#[cfg(target_os = "macos")]
fn present_in_chrome(clinic_prefix: &str, url: &str) -> Result<(), ChromePresentationError> {
    let navigation_script = spa_navigation_script(url).map_err(|_| ChromePresentationError::Unavailable)?;
    let output = Command::new("/usr/bin/osascript")
        .arg("-e")
        .arg(CHROME_PRESENT_SCRIPT)
        .arg("--")
        .arg(clinic_prefix)
        .arg(url)
        .arg(navigation_script)
        .stdin(Stdio::null())
        .stderr(Stdio::null())
        .output()
        .map_err(|_| ChromePresentationError::Unavailable)?;
    if !output.status.success() {
        return Err(ChromePresentationError::Unavailable);
    }
    let result = String::from_utf8_lossy(&output.stdout);
    if result.trim() == "javascript-disabled" {
        return Err(ChromePresentationError::JavascriptDisabled);
    }
    if result.trim() == "no-ready-tab" {
        return Err(ChromePresentationError::NoReadyTab);
    }
    Ok(())
}

#[cfg(not(target_os = "macos"))]
fn present_in_chrome(_clinic_prefix: &str, _url: &str) -> Result<(), ChromePresentationError> {
    Err(ChromePresentationError::Unavailable)
}

#[cfg(target_os = "macos")]
fn open_in_chrome(url: &str) -> Result<(), String> {
    Command::new("/usr/bin/open")
        .args(["-a", "Google Chrome", url])
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status()
        .map_err(|_| "Google Chrome could not start".to_string())?
        .success()
        .then_some(())
        .ok_or_else(|| "Google Chrome rejected the AIDOO URL".into())
}

#[cfg(not(target_os = "macos"))]
fn open_in_chrome(_url: &str) -> Result<(), String> {
    Err("Google Chrome presentation is not implemented on this platform".into())
}

#[cfg(target_os = "macos")]
fn open_in_default_browser(url: &str) -> Result<(), String> {
    Command::new("/usr/bin/open")
        .arg(url)
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status()
        .map_err(|_| "The default browser could not start".to_string())?
        .success()
        .then_some(())
        .ok_or_else(|| "The default browser rejected the AIDOO URL".into())
}

#[cfg(not(target_os = "macos"))]
fn open_in_default_browser(_url: &str) -> Result<(), String> {
    Err("Browser presentation is not implemented on this platform".into())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn builds_the_observed_status_and_treatment_routes() {
        let scheme = ["https:", "//"].concat();
        let clinic = format!("{scheme}aidoo-web.on.dev-craft.tech/clinics/demo/login");
        let status = patient_view_url(&clinic, "patient-id", PatientView::Status, 42).unwrap();
        let treatment =
            patient_view_url(&clinic, "patient-id", PatientView::Treatment, 43).unwrap();

        assert_eq!(
            status.clinic_prefix,
            format!("{scheme}aidoo-web.on.dev-craft.tech/clinics/demo/")
        );
        assert_eq!(
            status.url,
            format!(
                "{scheme}aidoo-web.on.dev-craft.tech/clinics/demo/medical-record?patientid=patient-id&tab=record&mode=status&selectedTeeth=&triggerNzokChecksProp=true&aidooControlSync=42"
            )
        );
        assert!(treatment.url.contains("mode=treatment"));
        assert!(treatment.url.ends_with("aidooControlSync=43"));
    }

    #[test]
    fn refuses_patient_ids_that_could_escape_the_query_value() {
        let clinic = ["https:", "//app.aidoo.bg/clinics/demo/login"].concat();
        assert!(
            patient_view_url(&clinic, "patient&mode=treatment", PatientView::Status, 1,).is_err()
        );
    }

    #[test]
    fn builds_the_observed_schedule_route_for_the_exact_date_and_doctor() {
        let scheme = ["https:", "//"].concat();
        let clinic = format!("{scheme}app.aidoo.bg/clinics/demo/login");
        let target = schedule_view_url(&clinic, "2026-09-21", "doctor-id", 44).unwrap();
        assert_eq!(
            target.url,
            format!("{scheme}app.aidoo.bg/clinics/demo/schedule?mode=doctors&active-date=2026-09-21&selected-doctors=%5B%22doctor-id%22%5D&aidooControlSync=44")
        );
    }

    #[test]
    fn refuses_schedule_values_that_can_escape_the_query() {
        let clinic = ["https:", "//app.aidoo.bg/clinics/demo/login"].concat();
        assert!(schedule_view_url(&clinic, "2026-09-21&mode=x", "doctor-id", 1).is_err());
        assert!(schedule_view_url(&clinic, "2026-09-21", "doctor&id", 1).is_err());
    }

    #[test]
    fn reuses_the_loaded_spa_without_hard_reloading_the_patient_route() {
        assert!(
            CHROME_PRESENT_SCRIPT.contains("execute browserTab javascript navigationJavaScript")
        );
        assert!(!CHROME_PRESENT_SCRIPT.contains("set URL of browserTab to targetURL"));
        assert!(CHROME_PRESENT_SCRIPT.contains("return \"no-ready-tab\""));

        let scheme = ["https:", "//"].concat();
        let target = format!(
            "{scheme}app.aidoo.bg/clinics/demo/medical-record?patientid=patient-id&mode=status"
        );
        let script = spa_navigation_script(&target).unwrap();
        assert!(script.contains("history.pushState"));
        assert!(script.contains("PopStateEvent('popstate')"));
        assert!(script.contains("return'navigated'"));
        assert!(script.contains(&format!("\"{target}\"")));
    }

    #[test]
    fn safely_encodes_the_target_inside_the_spa_navigation_script() {
        let scheme = ["https:", "//"].concat();
        let target = format!("{scheme}app.aidoo.bg/clinics/demo/?value=\"quoted\"");
        let script = spa_navigation_script(&target).unwrap();
        assert!(script.contains("value=\\\"quoted\\\""));
    }
}
