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
  tell application "Google Chrome"
    if (count of windows) is 0 then
      make new window
    end if
    repeat with browserWindow in windows
      repeat with tabIndex from 1 to (count of tabs of browserWindow)
        set browserTab to tab tabIndex of browserWindow
        set tabURL to URL of browserTab
        if tabURL starts with clinicPrefix then
          set URL of browserTab to targetURL
          set active tab index of browserWindow to tabIndex
          set index of browserWindow to 1
          activate
          return "reused"
        end if
      end repeat
    end repeat
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
    let thread_app = app.clone();
    let _ = std::thread::Builder::new()
        .name("aidoo-browser-presentation".into())
        .spawn(move || {
            if present_in_chrome(&target.clinic_prefix, &target.url).is_ok() {
                return;
            }
            if open_in_chrome(&target.url).is_ok() {
                return;
            }
            if open_in_default_browser(&target.url).is_ok() {
                return;
            }
            crate::storage::append_diagnostic("AIDOO browser presentation failed");
            let _ = thread_app.emit(
                "toast",
                "AIDOO промяната е запазена, но пациентският екран не можа да бъде показан.",
            );
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

fn sync_nonce() -> u128 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|duration| duration.as_millis())
        .unwrap_or(0)
}

#[cfg(target_os = "macos")]
fn present_in_chrome(clinic_prefix: &str, url: &str) -> Result<(), String> {
    let status = Command::new("/usr/bin/osascript")
        .arg("-e")
        .arg(CHROME_PRESENT_SCRIPT)
        .arg("--")
        .arg(clinic_prefix)
        .arg(url)
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status()
        .map_err(|_| "Chrome automation could not start".to_string())?;
    status
        .success()
        .then_some(())
        .ok_or_else(|| "Chrome automation was not available".into())
}

#[cfg(not(target_os = "macos"))]
fn present_in_chrome(_clinic_prefix: &str, _url: &str) -> Result<(), String> {
    Err("Chrome automation is not implemented on this platform".into())
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
}
