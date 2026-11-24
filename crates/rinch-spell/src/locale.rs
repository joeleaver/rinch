//! Locale tags as dictionary names.

/// Normalises a locale to a dictionary name: `en-US`, `en_US.UTF-8` and
/// `en_US@euro` all become `en_US`. A BCP 47 script subtag is dropped and
/// the region kept (`zh-Hans-CN` → `zh_CN`, `es-419` → `es_419`); variants
/// after the region are dropped. Returns `None` for `C`, `POSIX` or an empty
/// string.
pub fn normalize_locale(locale: &str) -> Option<String> {
    let base = locale.split(['.', '@']).next().unwrap_or("").trim();
    if base.is_empty() || base == "C" || base == "POSIX" {
        return None;
    }
    let mut parts = base.split(['-', '_']).filter(|p| !p.is_empty());
    let lang = parts.next()?.to_ascii_lowercase();
    let is_script = |p: &str| p.len() == 4 && p.chars().all(|c| c.is_ascii_alphabetic());
    let is_region = |p: &str| {
        (p.len() == 2 && p.chars().all(|c| c.is_ascii_alphabetic()))
            || (p.len() == 3 && p.chars().all(|c| c.is_ascii_digit()))
    };
    match parts.find(|p| !is_script(p)).filter(|p| is_region(p)) {
        Some(region) => Some(format!("{lang}_{}", region.to_ascii_uppercase())),
        None => Some(lang),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn locales_normalise_to_dictionary_names() {
        assert_eq!(normalize_locale("en-US").as_deref(), Some("en_US"));
        assert_eq!(normalize_locale("en_us.UTF-8").as_deref(), Some("en_US"));
        assert_eq!(normalize_locale("de_DE@euro").as_deref(), Some("de_DE"));
        assert_eq!(normalize_locale("FR").as_deref(), Some("fr"));
        // BCP 47 script subtags are dropped; the region is kept.
        assert_eq!(normalize_locale("zh-Hans-CN").as_deref(), Some("zh_CN"));
        assert_eq!(normalize_locale("zh-Hans").as_deref(), Some("zh"));
        assert_eq!(normalize_locale("sr-Latn-RS").as_deref(), Some("sr_RS"));
        assert_eq!(normalize_locale("es-419").as_deref(), Some("es_419"));
        assert_eq!(normalize_locale("C.UTF-8"), None);
        assert_eq!(normalize_locale("POSIX"), None);
        assert_eq!(normalize_locale(""), None);
    }
}
