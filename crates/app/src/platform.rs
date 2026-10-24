//! Windows-specific lookups. Everything that touches the OS lives here so the
//! rest of the app stays testable. All of it is read-only.

use std::path::PathBuf;

use vmm_core::game::{GameInstall, Sources, Store};

/// `VAEXIL_MOD_MANAGER_GAME` names a game folder to use instead of looking
/// for installs. Development and smoke-test runs use it to stay away from
/// the real game.
pub fn game_override() -> Option<GameInstall> {
    let dir = std::env::var_os("VAEXIL_MOD_MANAGER_GAME").filter(|d| !d.is_empty())?;
    Some(GameInstall { store: Store::Folder, root: PathBuf::from(dir) })
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

#[cfg(windows)]
pub fn is_minimized(hwnd: windows::Win32::Foundation::HWND) -> bool {
    // SAFETY: plain window-state query.
    unsafe { windows::Win32::UI::WindowsAndMessaging::IsIconic(hwnd).as_bool() }
}
