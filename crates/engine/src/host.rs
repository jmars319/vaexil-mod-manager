//! What the engine asks of Windows: where the game is (Steam, Epic, the
//! Microsoft Store), the app's own data folder, which processes are running,
//! and handing a link or file to Windows to open. Nothing here writes.

use std::path::PathBuf;

use vmm_core::error::{ErrorCode, VmmError};
use vmm_core::game::{GameInstall, Sources, Store};

/// `VAEXIL_MOD_MANAGER_GAME` names a game folder to use instead of looking
/// for installs. Development and smoke-test runs use it to stay away from
/// the real game.
pub fn game_override() -> Option<GameInstall> {
    let dir = std::env::var_os("VAEXIL_MOD_MANAGER_GAME").filter(|d| !d.is_empty())?;
    Some(GameInstall { store: Store::Folder, root: PathBuf::from(dir) })
}

/// The app's own folder, `%LOCALAPPDATA%\Vaexil Mod Manager`.
/// `VAEXIL_MOD_MANAGER_DATA` replaces it, so smoke tests stay out of the
/// user's profile.
pub fn data_dir() -> Option<PathBuf> {
    if let Some(dir) = std::env::var_os("VAEXIL_MOD_MANAGER_DATA").filter(|d| !d.is_empty()) {
        return Some(PathBuf::from(dir));
    }
    let base = std::env::var_os("LOCALAPPDATA").filter(|d| !d.is_empty())?;
    Some(PathBuf::from(base).join("Vaexil Mod Manager"))
}

pub fn sources() -> Sources {
    Sources { steam: steam_root(), epic_manifests: epic_manifests(), xbox_roots: xbox_roots() }
}

fn steam_root() -> Option<PathBuf> {
    registry_steam_path().map(PathBuf::from).or_else(|| {
        let default = PathBuf::from(r"C:\Program Files (x86)\Steam");
        default.is_dir().then_some(default)
    })
}

#[cfg(windows)]
fn registry_steam_path() -> Option<String> {
    use windows::Win32::System::Registry::{HKEY_CURRENT_USER, RRF_RT_REG_SZ, RegGetValueW};
    use windows::core::w;

    let mut buf = [0u16; 1024];
    let mut len = (buf.len() * 2) as u32;
    // SAFETY: the buffer and its length in bytes are valid for the call.
    let status = unsafe {
        RegGetValueW(
            HKEY_CURRENT_USER,
            w!(r"Software\Valve\Steam"),
            w!("SteamPath"),
            RRF_RT_REG_SZ,
            None,
            Some(buf.as_mut_ptr().cast()),
            Some(&mut len),
        )
    };
    if status.is_err() {
        return None;
    }
    let end = buf.iter().position(|&c| c == 0).unwrap_or(buf.len());
    let path = String::from_utf16_lossy(&buf[..end]);
    (!path.is_empty()).then_some(path)
}

#[cfg(not(windows))]
fn registry_steam_path() -> Option<String> {
    None
}

fn epic_manifests() -> Option<PathBuf> {
    let base = std::env::var_os("ProgramData").map(PathBuf::from)?;
    let dir = base.join(r"Epic\EpicGamesLauncher\Data\Manifests");
    dir.is_dir().then_some(dir)
}

/// `X:\XboxGames` on every drive that has one, where the Microsoft Store
/// installs games by default.
fn xbox_roots() -> Vec<PathBuf> {
    (b'A'..=b'Z').map(|d| PathBuf::from(format!("{}:\\XboxGames", d as char))).filter(|p| p.is_dir()).collect()
}

/// Which of `names` (executable file names, any case) are running now.
#[cfg(windows)]
pub fn running(names: &[&str]) -> Vec<String> {
    use windows::Win32::Foundation::CloseHandle;
    use windows::Win32::System::Diagnostics::ToolHelp::{
        CreateToolhelp32Snapshot, PROCESSENTRY32W, Process32FirstW, Process32NextW, TH32CS_SNAPPROCESS,
    };

    let mut found = Vec::new();
    // SAFETY: a process snapshot, read with a correctly sized entry and closed.
    unsafe {
        let Ok(snap) = CreateToolhelp32Snapshot(TH32CS_SNAPPROCESS, 0) else { return found };
        let mut entry = PROCESSENTRY32W { dwSize: std::mem::size_of::<PROCESSENTRY32W>() as u32, ..Default::default() };
        let mut ok = Process32FirstW(snap, &mut entry).is_ok();
        while ok {
            let end = entry.szExeFile.iter().position(|&c| c == 0).unwrap_or(entry.szExeFile.len());
            let exe = String::from_utf16_lossy(&entry.szExeFile[..end]);
            if names.iter().any(|n| n.eq_ignore_ascii_case(&exe)) && !found.contains(&exe) {
                found.push(exe);
            }
            ok = Process32NextW(snap, &mut entry).is_ok();
        }
        let _ = CloseHandle(snap);
    }
    found
}

#[cfg(not(windows))]
pub fn running(_names: &[&str]) -> Vec<String> {
    Vec::new()
}

/// Opens a link or a file with what Windows has registered for it, such as
/// `steam://rungameid/...` with Steam, or a log file with the text editor.
#[cfg(windows)]
pub fn open(target: &str) -> Result<(), VmmError> {
    use windows::Win32::UI::Shell::ShellExecuteW;
    use windows::Win32::UI::WindowsAndMessaging::SW_SHOWNORMAL;
    use windows::core::{HSTRING, w};

    let target_w = HSTRING::from(target);
    // SAFETY: plain shell call with valid strings.
    let result = unsafe { ShellExecuteW(None, w!("open"), &target_w, None, None, SW_SHOWNORMAL) };
    // ShellExecute reports success with a value above 32.
    if result.0 as isize > 32 {
        Ok(())
    } else {
        Err(VmmError::new(ErrorCode::LaunchUnsupported, format!("Windows could not open {target}.")))
    }
}

#[cfg(not(windows))]
pub fn open(target: &str) -> Result<(), VmmError> {
    Err(VmmError::new(ErrorCode::LaunchUnsupported, format!("Opening {target} is only supported on Windows.")))
}
