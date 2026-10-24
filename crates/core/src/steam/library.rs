//! Read-only lookups in a Steam installation.

use std::path::{Path, PathBuf};

use super::vdf::{self, Value};
use crate::paths::normalize;

const STEAMID64_BASE: u64 = 76_561_197_960_265_728;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AppInfo {
    pub app_id: u32,
    pub name: String,
    pub install_dir: PathBuf,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Account {
    pub account_id: u32,
    pub persona_name: Option<String>,
    pub most_recent: bool,
    pub localconfig: PathBuf,
}

impl Account {
    pub fn label(&self) -> String {
        match &self.persona_name {
            Some(n) => n.clone(),
            None => format!("Steam account {}", self.account_id),
        }
    }
}

#[derive(Debug, Clone)]
pub struct SteamRoot {
    pub path: PathBuf,
}

fn read_vdf(path: &Path) -> Option<Value> {
    let text = std::fs::read(path).ok()?;
    vdf::parse(&String::from_utf8_lossy(&text)).ok()
}

impl SteamRoot {
    pub fn new(path: impl Into<PathBuf>) -> Self {
        Self { path: path.into() }
    }

    pub fn library_folders(&self) -> Vec<PathBuf> {
        let mut out = vec![self.path.clone()];
        if let Some(doc) = read_vdf(&self.path.join("steamapps/libraryfolders.vdf")) {
            for lib in doc.get("libraryfolders").map(Value::children).unwrap_or_default() {
                if let Some(p) = lib.value.get("path").and_then(Value::as_str) {
                    let p = PathBuf::from(p);
                    if !out.iter().any(|o| normalize(o) == normalize(&p)) {
                        out.push(p);
                    }
                }
            }
        }
        out
    }

    pub fn installed_apps(&self) -> Vec<AppInfo> {
        let mut apps = Vec::new();
        for lib in self.library_folders() {
            let steamapps = lib.join("steamapps");
            let Ok(entries) = std::fs::read_dir(&steamapps) else { continue };
            for e in entries.flatten() {
                let name = e.file_name().to_string_lossy().to_lowercase();
                if !(name.starts_with("appmanifest_") && name.ends_with(".acf")) {
                    continue;
                }
                let Some(doc) = read_vdf(&e.path()) else { continue };
                let Some(state) = doc.get("AppState") else { continue };
                let (Some(id), Some(dir)) = (
                    state.get("appid").and_then(Value::as_str).and_then(|s| s.parse().ok()),
                    state.get("installdir").and_then(Value::as_str),
                ) else {
                    continue;
                };
                apps.push(AppInfo {
                    app_id: id,
                    name: state.get("name").and_then(Value::as_str).unwrap_or_default().to_string(),
                    install_dir: steamapps.join("common").join(dir),
                });
            }
        }
        apps
    }

    /// Finds the Steam app installed at `game_path` (MO2's `gamePath`).
    pub fn app_for_game_path(&self, game_path: &Path) -> Option<AppInfo> {
        let target = normalize(game_path);
        self.installed_apps().into_iter().find(|a| normalize(&a.install_dir) == target)
    }

    pub fn accounts(&self) -> Vec<Account> {
        let names = read_vdf(&self.path.join("config/loginusers.vdf"));
        let users = names.as_ref().and_then(|d| d.get("users"));
        let mut out: Vec<Account> = std::fs::read_dir(self.path.join("userdata"))
            .into_iter()
            .flatten()
            .flatten()
            .filter_map(|e| {
                let account_id: u32 = e.file_name().to_str()?.parse().ok()?;
                let localconfig = e.path().join("config/localconfig.vdf");
                if account_id == 0 || !localconfig.is_file() {
                    return None;
                }
                let user = users.and_then(|u| u.get(&(STEAMID64_BASE + u64::from(account_id)).to_string()));
                Some(Account {
                    account_id,
                    persona_name: user.and_then(|u| u.get("PersonaName")).and_then(Value::as_str).map(str::to_string),
                    most_recent: user.and_then(|u| u.get("MostRecent")).and_then(Value::as_str) == Some("1"),
                    localconfig,
                })
            })
            .collect();
        out.sort_by_key(|a| (!a.most_recent, a.account_id));
        out
    }

    /// Steam's locally cached library image for an app, such as
    /// `library_hero.jpg` or `logo.png`. Newer clients nest these one level
    /// down in a hashed folder.
    pub fn library_image(&self, app_id: u32, file: &str) -> Option<PathBuf> {
        let dir = self.path.join("appcache/librarycache").join(app_id.to_string());
        let direct = dir.join(file);
        if direct.is_file() {
            return Some(direct);
        }
        std::fs::read_dir(&dir).ok()?.flatten().map(|e| e.path().join(file)).find(|p| p.is_file())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn fake_steam() -> (tempfile::TempDir, SteamRoot) {
        let tmp = tempfile::tempdir().unwrap();
        let root = tmp.path().join("Steam");
        let lib2 = tmp.path().join("Library2");
        std::fs::create_dir_all(root.join("steamapps/common/Cyberpunk 2077")).unwrap();
        std::fs::create_dir_all(lib2.join("steamapps/common/The Witcher 3")).unwrap();
        std::fs::write(
            root.join("steamapps/libraryfolders.vdf"),
            format!(
                "\"libraryfolders\"\n{{\n\t\"0\"\n\t{{\n\t\t\"path\"\t\t\"{}\"\n\t}}\n\t\"1\"\n\t{{\n\t\t\"path\"\t\t\"{}\"\n\t}}\n}}\n",
                vdf::escape(&root.to_string_lossy()),
                vdf::escape(&lib2.to_string_lossy())
            ),
        )
        .unwrap();
        std::fs::write(root.join("steamapps/appmanifest_1091500.acf"), "\"AppState\"\n{\n\t\"appid\"\t\t\"1091500\"\n\t\"name\"\t\t\"Cyberpunk 2077\"\n\t\"installdir\"\t\t\"Cyberpunk 2077\"\n}\n").unwrap();
        std::fs::write(lib2.join("steamapps/appmanifest_292030.acf"), "\"AppState\"\n{\n\t\"appid\"\t\t\"292030\"\n\t\"name\"\t\t\"The Witcher 3: Wild Hunt\"\n\t\"installdir\"\t\t\"The Witcher 3\"\n}\n").unwrap();
        for id in ["96737131", "12345"] {
            std::fs::create_dir_all(root.join("userdata").join(id).join("config")).unwrap();
            std::fs::write(
                root.join("userdata").join(id).join("config/localconfig.vdf"),
                "\"UserLocalConfigStore\" { }",
            )
            .unwrap();
        }
        std::fs::create_dir_all(root.join("config")).unwrap();
        std::fs::write(
            root.join("config/loginusers.vdf"),
            format!("\"users\"\n{{\n\t\"{}\"\n\t{{\n\t\t\"AccountName\"\t\t\"acct\"\n\t\t\"PersonaName\"\t\t\"Jason\"\n\t\t\"MostRecent\"\t\t\"1\"\n\t}}\n}}\n", STEAMID64_BASE + 96737131),
        )
        .unwrap();
        let art = root.join("appcache/librarycache/1091500/abc123");
        std::fs::create_dir_all(&art).unwrap();
        std::fs::write(art.join("library_hero.jpg"), b"jpg").unwrap();
        (tmp, SteamRoot::new(root))
    }

    #[test]
    fn finds_apps_across_libraries() {
        let (_t, steam) = fake_steam();
        assert_eq!(steam.library_folders().len(), 2);
        let apps = steam.installed_apps();
        assert_eq!(apps.len(), 2);
        let w3 =
            steam.app_for_game_path(&apps.iter().find(|a| a.app_id == 292030).unwrap().install_dir.join("")).unwrap();
        assert_eq!(w3.name, "The Witcher 3: Wild Hunt");
        let cp = steam
            .app_for_game_path(Path::new(
                &steam.path.join("steamapps/common/cyberpunk 2077").to_string_lossy().replace('\\', "/"),
            ))
            .unwrap();
        assert_eq!(cp.app_id, 1091500);
        assert!(steam.app_for_game_path(Path::new(r"C:\Elsewhere")).is_none());
    }

    #[test]
    fn lists_accounts_most_recent_first() {
        let (_t, steam) = fake_steam();
        let accounts = steam.accounts();
        assert_eq!(accounts.len(), 2);
        assert_eq!(accounts[0].account_id, 96737131);
        assert_eq!(accounts[0].label(), "Jason");
        assert_eq!(accounts[1].label(), "Steam account 12345");
    }

    #[test]
    fn finds_nested_library_images() {
        let (_t, steam) = fake_steam();
        assert!(steam.library_image(1091500, "library_hero.jpg").is_some());
        assert!(steam.library_image(1091500, "logo.png").is_none());
        assert!(steam.library_image(292030, "library_hero.jpg").is_none());
    }
}
