use crate::DomainError;
use serde::{Deserialize, Serialize};

#[derive(Clone, Debug, Deserialize, Eq, Hash, PartialEq, Serialize)]
pub struct NormalizedPath {
    display: String,
    extended: String,
}

impl NormalizedPath {
    pub fn display(&self) -> &str {
        &self.display
    }

    pub fn extended(&self) -> &str {
        &self.extended
    }

    pub(crate) fn from_display_unchecked(display: String) -> Self {
        Self {
            extended: format!(r"\\?\{display}"),
            display,
        }
    }
}

pub fn normalize_path(input: &str) -> Result<NormalizedPath, DomainError> {
    if input.trim().is_empty() {
        return Err(DomainError::EmptyPath);
    }

    let path = input.replace('/', "\\");
    if path.starts_with(r"\\?\") || path.starts_with(r"\\.\") {
        return Err(DomainError::DevicePath);
    }
    if path.starts_with(r"\\") {
        return Err(DomainError::UnsupportedVolume);
    }

    let bytes = path.as_bytes();
    if bytes.len() < 3 || !bytes[0].is_ascii_alphabetic() || bytes[1] != b':' || bytes[2] != b'\\' {
        return Err(DomainError::UnsupportedVolume);
    }

    let mut components = Vec::new();
    for component in path[3..].split('\\').filter(|part| !part.is_empty()) {
        match component {
            "." => continue,
            ".." => {
                if components.pop().is_none() {
                    return Err(DomainError::InvalidPath);
                }
                continue;
            }
            _ => {}
        }

        if component.ends_with(['.', ' '])
            || component.chars().any(|character| {
                character.is_control()
                    || matches!(character, ':' | '<' | '>' | '"' | '|' | '?' | '*')
            })
        {
            return Err(DomainError::InvalidPath);
        }
        if is_reserved_name(component) {
            return Err(DomainError::ReservedName);
        }
        components.push(component);
    }

    let drive = (bytes[0] as char).to_ascii_uppercase();
    let display = if components.is_empty() {
        format!(r"{drive}:\")
    } else {
        format!(r"{drive}:\{}", components.join(r"\"))
    };

    Ok(NormalizedPath {
        extended: format!(r"\\?\{display}"),
        display,
    })
}

pub fn is_forbidden_path(path: &NormalizedPath) -> bool {
    let lower_path = path.display.to_lowercase();
    let components = path_components(path);

    if !is_explicit_cleanup_allowed(path) {
        return true;
    }

    let top = components[1].as_str();

    if top == "users"
        && !is_rebuildable_wechat_path(&components)
        && (components.len() <= 3
            || components.get(3).is_some_and(|part| {
                matches!(
                    part.as_str(),
                    "contacts"
                        | "desktop"
                        | "documents"
                        | "downloads"
                        | "favorites"
                        | "links"
                        | "music"
                        | "onedrive"
                        | "pictures"
                        | "saved games"
                        | "searches"
                        | "videos"
                )
            }))
    {
        return true;
    }

    components.iter().any(|part| {
        matches!(
            part.as_str(),
            "bootmgr"
                | "bookmarks"
                | "cookies"
                | "cookies.sqlite"
                | "extensions"
                | "hiberfil.sys"
                | "key4.db"
                | "login data"
                | "local state"
                | "logins.json"
                | "pagefile.sys"
                | "places.sqlite"
                | "$recycle.bin"
                | "sam"
                | "security"
                | "session storage"
                | "swapfile.sys"
                | "system"
                | "teams"
                | "weixin"
                | "web data"
        )
    }) || (components
        .iter()
        .any(|part| part == "wechat" || part == "wechat files")
        && !is_rebuildable_wechat_path(&components))
        || (lower_path.ends_with(".db") && !is_shell_cache_database(&components))
        || lower_path.ends_with(".sqlite")
        || lower_path.ends_with(".sqlite3")
        || lower_path.ends_with(".pst")
        || lower_path.ends_with(".ost")
}

pub fn is_explicit_cleanup_allowed(path: &NormalizedPath) -> bool {
    let lower_path = path.display.to_lowercase();
    let components = path_components(path);
    if components.len() < 2 {
        return false;
    }

    let top = components[1].as_str();
    let is_windows_error_report = top == "programdata"
        && components.len() > 6
        && components[2..5]
            == [
                "microsoft".to_owned(),
                "windows".to_owned(),
                "wer".to_owned(),
            ]
        && matches!(components[5].as_str(), "reportarchive" | "reportqueue");
    let system_path = &lower_path[3..];
    let is_delivery_cache =
        system_path.starts_with(r"programdata\microsoft\windows\deliveryoptimization\cache\");
    if matches!(top, "program files" | "program files (x86)")
        || (top == "programdata" && !is_windows_error_report && !is_delivery_cache)
        || (top == "windows" && !is_rebuildable_windows_file(system_path))
    {
        return false;
    }

    !components.iter().any(|part| {
        matches!(
            part.as_str(),
            "bootmgr"
                | "bookmarks"
                | "cookies"
                | "cookies.sqlite"
                | "extensions"
                | "hiberfil.sys"
                | "key4.db"
                | "login data"
                | "local state"
                | "logins.json"
                | "pagefile.sys"
                | "places.sqlite"
                | "$recycle.bin"
                | "sam"
                | "security"
                | "session storage"
                | "swapfile.sys"
                | "system"
                | "teams"
                | "weixin"
                | "web data"
        )
    }) && (!components
        .iter()
        .any(|part| part == "wechat" || part == "wechat files")
        || is_rebuildable_wechat_path(&components))
        && (!lower_path.ends_with(".db") || is_shell_cache_database(&components))
        && !lower_path.ends_with(".sqlite")
        && !lower_path.ends_with(".sqlite3")
        && !lower_path.ends_with(".pst")
        && !lower_path.ends_with(".ost")
}

fn is_rebuildable_windows_file(path: &str) -> bool {
    if path.starts_with(r"windows\temp\")
        || path.starts_with(r"windows\softwaredistribution\download\")
    {
        return true;
    }
    let extension = path.rsplit('.').next().unwrap_or("");
    if [
        r"windows\logs\",
        r"windows\system32\logfiles\",
        r"windows\panther\",
    ]
    .iter()
    .any(|root| path.starts_with(root))
    {
        return matches!(
            extension,
            "log" | "etl" | "cab" | "txt" | "old" | "bak" | "err" | "tmp"
        );
    }
    path == r"windows\memory.dmp"
        || (extension == "dmp"
            && [r"windows\minidump\", r"windows\livekernelreports\"]
                .iter()
                .any(|root| path.starts_with(root)))
        || (extension == "pf" && path.starts_with(r"windows\prefetch\"))
}

fn is_shell_cache_database(components: &[String]) -> bool {
    // Only shell thumbnail/icon databases are disposable; other .db files remain protected.
    components.len() == 9
        && components[1] == "users"
        && components[3..8] == ["appdata", "local", "microsoft", "windows", "explorer"]
        && (components[8].starts_with("thumbcache_") || components[8].starts_with("iconcache_"))
        && components[8].ends_with(".db")
}

fn is_rebuildable_wechat_path(components: &[String]) -> bool {
    let Some(marker) = components
        .iter()
        .position(|part| part == "wechat" || part == "wechat files")
    else {
        return false;
    };
    let Some(kind) = components.get(marker + 2) else {
        return false;
    };
    matches!(kind.as_str(), "log" | "cache")
        || (kind == "filestorage"
            && components.get(marker + 3).is_some_and(|item| {
                matches!(item.as_str(), "image" | "video" | "general" | "file")
            }))
}

fn path_components(path: &NormalizedPath) -> Vec<String> {
    path.display
        .split('\\')
        .filter(|part| !part.is_empty())
        .map(str::to_lowercase)
        .collect()
}

fn is_reserved_name(component: &str) -> bool {
    let stem = component.split('.').next().unwrap_or(component);
    let stem = stem.to_ascii_uppercase();

    matches!(
        stem.as_str(),
        "CON" | "PRN" | "AUX" | "NUL" | "COM¹" | "COM²" | "COM³" | "LPT¹" | "LPT²" | "LPT³"
    ) || stem
        .strip_prefix("COM")
        .or_else(|| stem.strip_prefix("LPT"))
        .is_some_and(|number| matches!(number, "1" | "2" | "3" | "4" | "5" | "6" | "7" | "8" | "9"))
}

#[cfg(test)]
mod tests {
    use super::{NormalizedPath, is_forbidden_path};

    #[test]
    fn malformed_internal_paths_fail_closed() {
        let path = NormalizedPath {
            display: String::new(),
            extended: String::new(),
        };

        assert!(is_forbidden_path(&path));
    }
}
