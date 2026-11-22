//! Hunspell dictionaries installed on the system, found by locale.
//!
//! Compiled with the `system-dictionaries` feature (on by default), never on
//! wasm32. The search path is Hunspell's own on Linux: `$DICPATH` first, then
//! the user's and the system's data directories (see [`search_dirs`]). A
//! dictionary is the pair `<dir>/<name>.aff` + `<dir>/<name>.dic`, named by
//! locale (`en_US`, `de_DE`, `fr`).

use std::env;
use std::path::{Path, PathBuf};

use crate::hunspell::{DictionaryError, HunspellDictionary};

/// The `.aff` and `.dic` files of one installed dictionary.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct DictionaryFiles {
    /// The dictionary's name, the file stem (`en_US`).
    pub name: String,
    /// The affix file.
    pub aff: PathBuf,
    /// The word list.
    pub dic: PathBuf,
}

impl DictionaryFiles {
    /// Reads and parses the pair.
    pub fn load(&self) -> Result<HunspellDictionary, DictionaryError> {
        let read = |path: &Path| {
            std::fs::read(path).map_err(|e| DictionaryError::Io(format!("{}: {e}", path.display())))
        };
        HunspellDictionary::from_bytes(self.name.clone(), &read(&self.aff)?, &read(&self.dic)?)
    }
}

/// The directories searched, in order: each entry of `$DICPATH`, then
/// `$XDG_DATA_HOME/hunspell` (`~/.local/share/hunspell`), then `hunspell`,
/// `myspell` and `myspell/dicts` under each `$XDG_DATA_DIRS` entry
/// (`/usr/local/share:/usr/share` when unset). Directories that do not exist
/// are listed too; lookups skip them.
pub fn search_dirs() -> Vec<PathBuf> {
    let mut dirs = Vec::new();
    if let Some(dicpath) = env::var_os("DICPATH") {
        dirs.extend(env::split_paths(&dicpath).filter(|p| !p.as_os_str().is_empty()));
    }
    let data_home = env::var_os("XDG_DATA_HOME")
        .filter(|v| !v.is_empty())
        .map(PathBuf::from)
        .or_else(|| env::var_os("HOME").map(|home| Path::new(&home).join(".local/share")));
    if let Some(data_home) = data_home {
        dirs.push(data_home.join("hunspell"));
    }
    let data_dirs = env::var_os("XDG_DATA_DIRS")
        .filter(|v| !v.is_empty())
        .unwrap_or_else(|| "/usr/local/share:/usr/share".into());
    for dir in env::split_paths(&data_dirs).filter(|p| !p.as_os_str().is_empty()) {
        dirs.push(dir.join("hunspell"));
        dirs.push(dir.join("myspell"));
        dirs.push(dir.join("myspell/dicts"));
    }
    dirs.dedup();
    dirs
}

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

/// The user's locales from the environment, most preferred first:
/// `$LANGUAGE` (colon-separated), then `$LC_ALL`, `$LC_MESSAGES`, `$LANG`;
/// normalised, `C`/`POSIX` dropped, without repeats.
pub fn user_locales() -> Vec<String> {
    let mut out: Vec<String> = Vec::new();
    let mut push = |value: &str| {
        if let Some(locale) = normalize_locale(value)
            && !out.contains(&locale)
        {
            out.push(locale);
        }
    };
    if let Ok(language) = env::var("LANGUAGE") {
        language.split(':').for_each(&mut push);
    }
    for var in ["LC_ALL", "LC_MESSAGES", "LANG"] {
        if let Ok(value) = env::var(var) {
            push(&value);
        }
    }
    out
}

/// Finds the dictionary for `locale` in `dirs` (in order). An exact name wins
/// (`en_US`); a region with no dictionary of its own falls back to the bare
/// language (`en`); a bare language with no file of that name takes the first
/// regional dictionary in name order (`de` → `de_AT`).
pub fn find_in(dirs: &[PathBuf], locale: &str) -> Option<DictionaryFiles> {
    let name = normalize_locale(locale)?;
    let lang = name.split('_').next().unwrap_or(&name).to_owned();
    let exact = |name: &str| dirs.iter().find_map(|dir| pair(dir, name));
    exact(&name)
        .or_else(|| (lang != name).then(|| exact(&lang)).flatten())
        .or_else(|| {
            let prefix = format!("{lang}_");
            let mut candidates: Vec<DictionaryFiles> = list_in(dirs)
                .into_iter()
                .filter(|f| f.name.starts_with(&prefix))
                .collect();
            candidates.sort_by(|a, b| a.name.cmp(&b.name));
            candidates.into_iter().next()
        })
}

/// [`find_in`] over [`search_dirs`].
pub fn find(locale: &str) -> Option<DictionaryFiles> {
    find_in(&search_dirs(), locale)
}

/// Finds and loads the dictionary for `locale`.
pub fn load(locale: &str) -> Result<HunspellDictionary, DictionaryError> {
    find(locale)
        .ok_or_else(|| DictionaryError::NotFound(locale.to_owned()))?
        .load()
}

/// Every dictionary in `dirs`, the first of each name winning, sorted by name.
pub fn list_in(dirs: &[PathBuf]) -> Vec<DictionaryFiles> {
    let mut out: Vec<DictionaryFiles> = Vec::new();
    for dir in dirs {
        let Ok(entries) = std::fs::read_dir(dir) else {
            continue;
        };
        let mut names: Vec<String> = entries
            .filter_map(|e| e.ok())
            .filter_map(|e| {
                let path = e.path();
                (path.extension()? == "aff")
                    .then(|| path.file_stem()?.to_str().map(str::to_owned))?
            })
            .collect();
        names.sort();
        for name in names {
            if out.iter().any(|f| f.name == name) {
                continue;
            }
            if let Some(files) = pair(dir, &name) {
                out.push(files);
            }
        }
    }
    out.sort_by(|a, b| a.name.cmp(&b.name));
    out
}

/// [`list_in`] over [`search_dirs`].
pub fn list() -> Vec<DictionaryFiles> {
    list_in(&search_dirs())
}

fn pair(dir: &Path, name: &str) -> Option<DictionaryFiles> {
    let aff = dir.join(format!("{name}.aff"));
    let dic = dir.join(format!("{name}.dic"));
    (aff.is_file() && dic.is_file()).then(|| DictionaryFiles {
        name: name.to_owned(),
        aff,
        dic,
    })
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
