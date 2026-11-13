//! Settings' "Add the command line to my PATH" (ADR 0003, step 2). Only the
//! window changes PATH, and only when the user asks after seeing exactly what
//! will change: the folder that holds `vmm.exe` is added to the end of the
//! user's own `Path` (`HKEY_CURRENT_USER\Environment`), or taken out again.
//! The system PATH is never touched, and nothing else in the value changes.

use std::path::{Path, PathBuf};

/// Where `vmm.exe` is: beside this app, or `None` when it isn't there.
pub fn cli_exe() -> Option<PathBuf> {
    let exe = std::env::current_exe().ok()?.parent()?.join("vmm.exe");
    exe.is_file().then_some(exe)
}

fn same_folder(entry: &str, folder: &str) -> bool {
    let tidy = |s: &str| s.trim().trim_end_matches(['\\', '/']).to_lowercase();
    !entry.trim().is_empty() && tidy(entry) == tidy(folder)
}

pub fn contains(path_value: &str, folder: &str) -> bool {
    path_value.split(';').any(|e| same_folder(e, folder))
}

/// `path_value` with `folder` added at the end, or unchanged when it is there.
pub fn with_folder(path_value: &str, folder: &str) -> String {
    if contains(path_value, folder) {
        return path_value.to_string();
    }
    let base = path_value.trim_end_matches(';');
    if base.is_empty() { folder.to_string() } else { format!("{base};{folder}") }
}

/// `path_value` without `folder`; every other entry stays as it was.
pub fn without_folder(path_value: &str, folder: &str) -> String {
    path_value.split(';').filter(|e| !same_folder(e, folder)).collect::<Vec<_>>().join(";")
}

#[cfg(windows)]
mod registry {
    use windows::Win32::Foundation::{ERROR_FILE_NOT_FOUND, LPARAM, WPARAM};
    use windows::Win32::System::Registry::{
        HKEY_CURRENT_USER, REG_EXPAND_SZ, REG_VALUE_TYPE, RRF_NOEXPAND, RRF_RT_REG_EXPAND_SZ, RRF_RT_REG_SZ,
        RegGetValueW, RegSetKeyValueW,
    };
    use windows::Win32::UI::WindowsAndMessaging::{
        HWND_BROADCAST, SMTO_ABORTIFHUNG, SendMessageTimeoutW, WM_SETTINGCHANGE,
    };
    use windows::core::w;

    /// The user's own `Path` and its registry type; empty when there is none.
    pub fn read() -> Result<(String, REG_VALUE_TYPE), String> {
        let mut kind = REG_VALUE_TYPE::default();
        let mut len = 0u32;
        let flags = RRF_RT_REG_SZ | RRF_RT_REG_EXPAND_SZ | RRF_NOEXPAND;
        // SAFETY: asks only for the size; the out pointers are valid.
        let status = unsafe {
            RegGetValueW(HKEY_CURRENT_USER, w!("Environment"), w!("Path"), flags, Some(&mut kind), None, Some(&mut len))
        };
        if status == ERROR_FILE_NOT_FOUND {
            return Ok((String::new(), REG_EXPAND_SZ));
        }
        status.ok().map_err(|e| format!("Couldn't read your PATH: {e}"))?;
        let mut buf = vec![0u16; (len as usize).div_ceil(2) + 1];
        let mut len = (buf.len() * 2) as u32;
        // SAFETY: the buffer and its length in bytes are valid for the call.
        unsafe {
            RegGetValueW(
                HKEY_CURRENT_USER,
                w!("Environment"),
                w!("Path"),
                flags,
                Some(&mut kind),
                Some(buf.as_mut_ptr().cast()),
                Some(&mut len),
            )
        }
        .ok()
        .map_err(|e| format!("Couldn't read your PATH: {e}"))?;
        let end = buf.iter().position(|&c| c == 0).unwrap_or(buf.len());
        Ok((String::from_utf16_lossy(&buf[..end]), kind))
    }

    /// Saves the user's `Path` with the same registry type, then tells
    /// running programs (Explorer, new terminals) that it changed.
    pub fn write(value: &str, kind: REG_VALUE_TYPE) -> Result<(), String> {
        let wide: Vec<u16> = value.encode_utf16().chain(std::iter::once(0)).collect();
        // SAFETY: the data pointer and its byte length describe `wide`.
        unsafe {
            RegSetKeyValueW(
                HKEY_CURRENT_USER,
                w!("Environment"),
                w!("Path"),
                kind.0,
                Some(wide.as_ptr().cast()),
                (wide.len() * 2) as u32,
            )
        }
        .ok()
        .map_err(|e| format!("Couldn't save your PATH: {e}"))?;
        // SAFETY: a broadcast with a static string; it waits at most 2 seconds.
        unsafe {
            SendMessageTimeoutW(
                HWND_BROADCAST,
                WM_SETTINGCHANGE,
                WPARAM(0),
                LPARAM(w!("Environment").as_ptr() as isize),
                SMTO_ABORTIFHUNG,
                2000,
                None,
            );
        }
        Ok(())
    }
}

/// Whether `folder` is on the user's PATH.
#[cfg(windows)]
pub fn on_user_path(folder: &Path) -> Result<bool, String> {
    Ok(contains(&registry::read()?.0, &folder.display().to_string()))
}

/// Adds `folder` to (or with `add` false, takes it out of) the user's PATH.
#[cfg(windows)]
pub fn change_user_path(folder: &Path, add: bool) -> Result<(), String> {
    let (value, kind) = registry::read()?;
    let folder = folder.display().to_string();
    let new = if add { with_folder(&value, &folder) } else { without_folder(&value, &folder) };
    if new != value { registry::write(&new, kind) } else { Ok(()) }
}

#[cfg(not(windows))]
pub fn on_user_path(_folder: &Path) -> Result<bool, String> {
    Err("PATH can only be changed on Windows.".into())
}

#[cfg(not(windows))]
pub fn change_user_path(_folder: &Path, _add: bool) -> Result<(), String> {
    Err("PATH can only be changed on Windows.".into())
}

#[cfg(test)]
mod tests {
    use super::*;

    const DIR: &str = r"C:\Users\me\AppData\Local\Programs\Vaexil Mod Manager";

    #[test]
    fn adding_appends_once_and_keeps_the_rest() {
        assert_eq!(with_folder("", DIR), DIR);
        assert_eq!(with_folder(r"%USERPROFILE%\bin;", DIR), format!(r"%USERPROFILE%\bin;{DIR}"));
        let once = with_folder(r"C:\a", DIR);
        assert_eq!(with_folder(&once, DIR), once);
        // The same folder written differently counts as there.
        assert!(contains(&format!(r"C:\a;{}\", DIR.to_uppercase()), DIR));
    }

    #[test]
    fn removing_takes_out_only_that_folder() {
        let value = format!(r"C:\a;{DIR};%USERPROFILE%\bin;{DIR}\");
        assert_eq!(without_folder(&value, DIR), r"C:\a;%USERPROFILE%\bin");
        assert_eq!(without_folder(r"C:\a;;C:\b", DIR), r"C:\a;;C:\b");
        assert!(!contains(r"C:\a;;C:\b", ""));
    }
}
