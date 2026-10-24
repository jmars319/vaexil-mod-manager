//! Finding HITMAN World of Assassination installs. Read-only.

use std::path::{Path, PathBuf};

use serde::Deserialize;

use crate::paths::{normalize, strip_verbatim};
use crate::steam::SteamRoot;

/// HITMAN World of Assassination on Steam.
pub const STEAM_APP_ID: u32 = 1_659_040;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Store {
    Steam,
    Epic,
    Microsoft,
    /// A folder named by `VAEXIL_MOD_MANAGER_GAME`, for development and tests.
    Folder,
}

impl Store {
    pub fn label(self) -> &'static str {
        match self {
            Store::Steam => "Steam",
            Store::Epic => "Epic Games",
            Store::Microsoft => "Microsoft Store",
            Store::Folder => "Folder",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct GameInstall {
    pub store: Store,
    /// The folder that holds `Retail`. SMF lives in a folder directly inside it.
    pub root: PathBuf,
}

/// Where to look. The app fills this from the registry and the drive list;
/// tests point it at temporary folders.
#[derive(Debug, Default, Clone)]
pub struct Sources {
    pub steam: Option<PathBuf>,
    pub epic_manifests: Option<PathBuf>,
    /// `XboxGames` folders, one per drive that has one.
    pub xbox_roots: Vec<PathBuf>,
}

/// True when `root` holds HITMAN's `Retail` folder with the game in it. This
/// mirrors the check SMF's own Mod Manager makes before it will deploy.
pub fn is_game_root(root: &Path) -> bool {
    let retail = root.join("Retail");
    retail.join("HITMAN3.exe").is_file() || retail.join("Runtime").join("chunk0.rpkg").is_file()
}

/// Every HITMAN install found, Steam first, without duplicates.
pub fn find_installs(sources: &Sources) -> Vec<GameInstall> {
    let mut found: Vec<GameInstall> = Vec::new();
    let mut push = |store: Store, root: PathBuf| {
        if is_game_root(&root) && !found.iter().any(|g| normalize(&g.root) == normalize(&root)) {
            found.push(GameInstall { store, root: display_path(root) });
        }
    };
    if let Some(steam) = &sources.steam {
        for app in SteamRoot::new(steam).installed_apps() {
            if app.app_id == STEAM_APP_ID {
                push(Store::Steam, app.install_dir);
            }
        }
    }
    if let Some(dir) = &sources.epic_manifests {
        for root in epic_installs(dir) {
            push(Store::Epic, root);
        }
    }
    for xbox in &sources.xbox_roots {
        for root in microsoft_installs(xbox) {
            push(Store::Microsoft, root);
        }
    }
    found
}

/// The folder as Windows spells it. Steam's registry value is lowercase with
/// forward slashes, which reads badly in the window.
fn display_path(path: PathBuf) -> PathBuf {
    std::fs::canonicalize(&path).map(strip_verbatim).unwrap_or(path)
}

#[derive(Deserialize)]
struct EpicItem {
    #[serde(rename = "DisplayName", default)]
    display_name: String,
    #[serde(rename = "InstallLocation", default)]
    install_location: String,
}

/// Epic writes one `.item` JSON file per installed game.
fn epic_installs(manifests: &Path) -> Vec<PathBuf> {
    let Ok(entries) = std::fs::read_dir(manifests) else { return Vec::new() };
    entries
        .flatten()
        .map(|e| e.path())
        .filter(|p| p.extension().is_some_and(|x| x.eq_ignore_ascii_case("item")))
        .filter_map(|p| serde_json::from_slice::<EpicItem>(&std::fs::read(p).ok()?).ok())
        .filter(|i| i.display_name.to_lowercase().contains("hitman") && !i.install_location.is_empty())
        .map(|i| PathBuf::from(i.install_location))
        .collect()
}

/// The Microsoft Store installs games as `XboxGames\<name>\Content`, with
/// `MicrosoftGame.Config` beside `Retail`.
fn microsoft_installs(xbox_root: &Path) -> Vec<PathBuf> {
    let Ok(entries) = std::fs::read_dir(xbox_root) else { return Vec::new() };
    entries
        .flatten()
        .map(|e| e.path().join("Content"))
        .filter(|content| content.join("MicrosoftGame.Config").is_file())
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn make_game(root: &Path) {
        std::fs::create_dir_all(root.join("Retail")).unwrap();
        std::fs::write(root.join("Retail/HITMAN3.exe"), b"").unwrap();
    }

    #[test]
    fn recognises_a_game_root() {
        let tmp = tempfile::tempdir().unwrap();
        assert!(!is_game_root(tmp.path()));
        make_game(tmp.path());
        assert!(is_game_root(tmp.path()));

        let ms = tempfile::tempdir().unwrap();
        std::fs::create_dir_all(ms.path().join("Retail/Runtime")).unwrap();
        std::fs::write(ms.path().join("Retail/Runtime/chunk0.rpkg"), b"").unwrap();
        assert!(is_game_root(ms.path()));
    }

    #[test]
    fn finds_steam_epic_and_microsoft_installs() {
        let tmp = tempfile::tempdir().unwrap();
        let steam = tmp.path().join("Steam");
        let steam_game = steam.join("steamapps/common/HITMAN 3");
        make_game(&steam_game);
        std::fs::write(
            steam.join("steamapps/appmanifest_1659040.acf"),
            "\"AppState\"\n{\n\t\"appid\"\t\t\"1659040\"\n\t\"name\"\t\t\"HITMAN World of Assassination\"\n\t\"installdir\"\t\t\"HITMAN 3\"\n}\n",
        )
        .unwrap();
        std::fs::write(
            steam.join("steamapps/appmanifest_10.acf"),
            "\"AppState\" { \"appid\" \"10\" \"name\" \"Other\" \"installdir\" \"Other\" }",
        )
        .unwrap();

        let epic_game = tmp.path().join("Epic/HITMAN3");
        make_game(&epic_game);
        let manifests = tmp.path().join("Manifests");
        std::fs::create_dir_all(&manifests).unwrap();
        std::fs::write(
            manifests.join("a.item"),
            serde_json::json!({ "DisplayName": "HITMAN World of Assassination", "InstallLocation": epic_game })
                .to_string(),
        )
        .unwrap();
        std::fs::write(
            manifests.join("b.item"),
            serde_json::json!({ "DisplayName": "Another Game", "InstallLocation": epic_game }).to_string(),
        )
        .unwrap();

        let xbox = tmp.path().join("XboxGames");
        let ms_game = xbox.join("HITMAN World of Assassination/Content");
        make_game(&ms_game);
        std::fs::write(ms_game.join("MicrosoftGame.Config"), b"<Game/>").unwrap();
        std::fs::create_dir_all(xbox.join("GameSave")).unwrap();

        let found = find_installs(&Sources {
            steam: Some(steam),
            epic_manifests: Some(manifests),
            xbox_roots: vec![xbox, tmp.path().join("missing")],
        });
        let stores: Vec<Store> = found.iter().map(|g| g.store).collect();
        assert_eq!(stores, [Store::Steam, Store::Epic, Store::Microsoft]);
        // Found folders are reported as Windows spells them, which expands
        // 8.3 short names such as CI's RUNNER~1, so compare against that.
        assert_eq!(normalize(&found[0].root), normalize(&display_path(steam_game)));
        assert!(!found[0].root.to_string_lossy().starts_with(r"\\?\"));
    }

    #[test]
    fn reports_steam_paths_as_windows_spells_them() {
        let tmp = tempfile::tempdir().unwrap();
        let steam = tmp.path().join("Steam");
        make_game(&steam.join("steamapps/common/HITMAN 3"));
        std::fs::write(
            steam.join("steamapps/appmanifest_1659040.acf"),
            "\"AppState\" { \"appid\" \"1659040\" \"name\" \"HITMAN\" \"installdir\" \"HITMAN 3\" }",
        )
        .unwrap();
        // Steam's registry value looks like this: lowercase, forward slashes.
        let registry_style = PathBuf::from(steam.to_string_lossy().to_lowercase().replace('\\', "/"));
        let found = find_installs(&Sources { steam: Some(registry_style), ..Default::default() });
        let shown = found[0].root.to_string_lossy().into_owned();
        assert!(!shown.contains('/'), "{shown}");
        assert!(shown.ends_with(r"steamapps\common\HITMAN 3"), "{shown}");
    }

    #[test]
    fn skips_listed_installs_that_are_not_on_disk() {
        let tmp = tempfile::tempdir().unwrap();
        let manifests = tmp.path().join("Manifests");
        std::fs::create_dir_all(&manifests).unwrap();
        std::fs::write(
            manifests.join("a.item"),
            serde_json::json!({ "DisplayName": "HITMAN 3", "InstallLocation": tmp.path().join("gone") }).to_string(),
        )
        .unwrap();
        assert!(find_installs(&Sources { epic_manifests: Some(manifests), ..Default::default() }).is_empty());
    }
}
