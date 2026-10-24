use std::path::{Path, PathBuf};

/// Normalizes a Windows path for comparison: backslashes, no verbatim prefix,
/// no trailing separator, lowercase. Windows paths are case-insensitive.
pub fn normalize(path: &Path) -> String {
    let raw = path.to_string_lossy().replace('/', "\\");
    let mut s = if let Some(rest) = raw.strip_prefix(r"\\?\UNC\") {
        format!(r"\\{rest}")
    } else if let Some(rest) = raw.strip_prefix(r"\\?\") {
        rest.to_string()
    } else {
        raw
    };
    while s.len() > 3 && s.ends_with('\\') {
        s.pop();
    }
    s.to_lowercase()
}

pub fn same_path(a: &Path, b: &Path) -> bool {
    normalize(a) == normalize(b)
}

/// Removes the `\\?\` prefix that `std::fs::canonicalize` adds on Windows.
pub fn strip_verbatim(path: PathBuf) -> PathBuf {
    let s = path.to_string_lossy();
    if let Some(rest) = s.strip_prefix(r"\\?\UNC\") {
        PathBuf::from(format!(r"\\{rest}"))
    } else if let Some(rest) = s.strip_prefix(r"\\?\") {
        PathBuf::from(rest)
    } else {
        path
    }
}

pub fn file_name_eq(path: &Path, name: &str) -> bool {
    path.file_name().and_then(|n| n.to_str()).is_some_and(|n| n.eq_ignore_ascii_case(name))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn normalize_handles_case_slashes_and_prefixes() {
        assert_eq!(
            normalize(Path::new(r"\\?\C:\Modlists\ProjectUltrapunk\ModOrganizer.exe")),
            normalize(Path::new("c:/modlists/projectultrapunk/modorganizer.exe"))
        );
        assert_eq!(normalize(Path::new(r"C:\Games\")), r"c:\games");
        assert_eq!(normalize(Path::new(r"C:\")), r"c:\");
        assert_eq!(normalize(Path::new(r"\\?\UNC\server\share\x")), r"\\server\share\x");
    }

    #[test]
    fn strip_verbatim_removes_prefix() {
        assert_eq!(strip_verbatim(PathBuf::from(r"\\?\C:\a")), PathBuf::from(r"C:\a"));
        assert_eq!(strip_verbatim(PathBuf::from(r"C:\a")), PathBuf::from(r"C:\a"));
    }

    #[test]
    fn file_name_eq_is_case_insensitive() {
        assert!(file_name_eq(Path::new(r"C:\x\modorganizer.EXE"), "ModOrganizer.exe"));
        assert!(!file_name_eq(Path::new(r"C:\x\helper.exe"), "ModOrganizer.exe"));
    }
}
