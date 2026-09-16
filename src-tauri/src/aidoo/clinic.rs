pub const AIDOO_CLINIC_WEB_ORIGIN: &str = "https://app.aidoo.bg";
const AIDOO_CLINIC_API_BASE: &str = "https://app.aidoo.bg/web";
const AIDOO_TEST_WEB_ORIGIN: &str = "https://aidoo-web.on.dev-craft.tech";
const AIDOO_TEST_API_BASE: &str = "https://aidoo-platform.on.dev-craft.tech/web";

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ClinicReference {
    pub slug: String,
    pub url: String,
    pub api_base: String,
}

pub fn parse_clinic_reference(value: &str) -> Result<ClinicReference, String> {
    let value = value.trim().trim_end_matches('/');
    if value.is_empty() {
        return Err("Въведете линк към AIDOO клиниката.".into());
    }

    let (slug, web_origin, api_base) = if value.contains("://") {
        let production_prefix = format!("{AIDOO_CLINIC_WEB_ORIGIN}/clinics/");
        let test_prefix = format!("{AIDOO_TEST_WEB_ORIGIN}/clinics/");
        let (path, web_origin, api_base) = if let Some(path) =
            value.strip_prefix(&production_prefix)
        {
            (path, AIDOO_CLINIC_WEB_ORIGIN, AIDOO_CLINIC_API_BASE)
        } else if let Some(path) = value.strip_prefix(&test_prefix) {
            (path, AIDOO_TEST_WEB_ORIGIN, AIDOO_TEST_API_BASE)
        } else {
            return Err(format!(
                "Линкът трябва да е към клиника в AIDOO, например {AIDOO_CLINIC_WEB_ORIGIN}/clinics/demo/login."
            ));
        };
        let slug = path.split(['/', '?', '#']).next().unwrap_or_default();
        (slug, web_origin, api_base)
    } else {
        // Keep existing saved installations compatible with the former clinic-slug field.
        (value, AIDOO_TEST_WEB_ORIGIN, AIDOO_TEST_API_BASE)
    };

    if slug.is_empty()
        || slug.len() > 128
        || !slug
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'_'))
    {
        return Err("Линкът към AIDOO клиниката е невалиден.".into());
    }

    Ok(ClinicReference {
        slug: slug.into(),
        url: format!("{web_origin}/clinics/{slug}/login"),
        api_base: api_base.into(),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn accepts_aidoo_clinic_links_and_uses_the_full_origin() {
        let reference = parse_clinic_reference(&format!(
            "{AIDOO_CLINIC_WEB_ORIGIN}/clinics/demo/patients/123"
        ))
        .unwrap();
        assert_eq!(reference.slug, "demo");
        assert_eq!(
            reference.url,
            format!("{AIDOO_CLINIC_WEB_ORIGIN}/clinics/demo/login")
        );
        assert_eq!(reference.api_base, AIDOO_CLINIC_API_BASE);
    }

    #[test]
    fn distinguishes_test_and_production_from_the_full_link() {
        let reference = parse_clinic_reference(&format!(
            "{AIDOO_TEST_WEB_ORIGIN}/clinics/demo/calendar?view=day"
        ))
        .unwrap();
        assert_eq!(reference.slug, "demo");
        assert_eq!(
            reference.url,
            format!("{AIDOO_TEST_WEB_ORIGIN}/clinics/demo/login")
        );
        assert_eq!(reference.api_base, AIDOO_TEST_API_BASE);
    }

    #[test]
    fn keeps_legacy_slugs_compatible() {
        let reference = parse_clinic_reference("demo").unwrap();
        assert_eq!(reference.slug, "demo");
        assert_eq!(
            reference.url,
            format!("{AIDOO_TEST_WEB_ORIGIN}/clinics/demo/login")
        );
        assert_eq!(reference.api_base, AIDOO_TEST_API_BASE);
    }

    #[test]
    fn rejects_external_or_malformed_links() {
        let external = ["https://", "example.com/clinics/demo/login"].concat();
        assert!(parse_clinic_reference(&external).is_err());
        let malformed = format!("{AIDOO_TEST_WEB_ORIGIN}/clinics/../../login");
        assert!(parse_clinic_reference(&malformed).is_err());
        assert!(parse_clinic_reference("").is_err());
    }
}
