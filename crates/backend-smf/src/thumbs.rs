//! A stale `cleanThumbs.dat` after a game update (ADR 0003, E7).
//!
//! SMF keeps a clean copy of the game's `Retail\thumbs.dat` as
//! `cleanThumbs.dat` and builds patched thumbs from it when "skip intro" is on
//! or a mod patches thumbs. Its updater keeps that copy, and a deploy never
//! refreshes it, so after a game update it can describe the old build
//! (found on JASON-PC: BuildID 8337407 against the game's 10814523). SMF's
//! own fallback, when the copy is missing, is to copy `Retail\thumbs.dat`
//! again. So a copy whose BuildID is older than the game's is set aside
//! (with a backup) and SMF recreates it at the next deploy.
//!
//! Both files are enciphered. They are read with SMF's own
//! `Third-Party\h6xtea.exe -d`, which writes the text to a file in Windows'
//! temporary folder; that file is deleted as soon as it is read.

use std::path::Path;

/// The `BuildID=` line of a deciphered thumbs.dat.
pub fn build_id(text: &str) -> Option<u64> {
    text.lines().find_map(|l| l.trim().strip_prefix("BuildID=")?.trim().parse().ok())
}

/// Deciphers `file` with SMF's `h6xtea.exe` and reads its BuildID.
pub fn read_build_id(h6xtea: &Path, file: &Path) -> Option<u64> {
    use std::sync::atomic::{AtomicU32, Ordering};
    static N: AtomicU32 = AtomicU32::new(0);
    let out_file = std::env::temp_dir().join(format!(
        "vaexil-mm-thumbs-{}-{}.txt",
        std::process::id(),
        N.fetch_add(1, Ordering::Relaxed)
    ));
    let mut cmd = std::process::Command::new(h6xtea);
    cmd.arg("-d")
        .arg("--src")
        .arg(file)
        .arg("--dst")
        .arg(&out_file)
        .stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null());
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        const CREATE_NO_WINDOW: u32 = 0x0800_0000;
        cmd.creation_flags(CREATE_NO_WINDOW);
    }
    let ok = cmd.status().is_ok_and(|s| s.success());
    let text = std::fs::read(&out_file).ok();
    let _ = std::fs::remove_file(&out_file);
    ok.then_some(())?;
    build_id(&String::from_utf8_lossy(&text?))
}

/// The game builds of SMF's clean copy and of the game's own thumbs.dat,
/// when the copy is older.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Stale {
    pub clean: u64,
    pub game: u64,
}

/// Whether SMF's `cleanThumbs.dat` is from an older game build, with `read`
/// giving a file's BuildID.
pub fn stale_with(smf_folder: &Path, game_root: &Path, read: impl Fn(&Path) -> Option<u64>) -> Option<Stale> {
    let clean = smf_folder.join("cleanThumbs.dat");
    let retail = game_root.join("Retail").join("thumbs.dat");
    if !clean.is_file() || !retail.is_file() {
        return None;
    }
    let (clean, game) = (read(&clean)?, read(&retail)?);
    (game > clean).then_some(Stale { clean, game })
}

/// [`stale_with`], reading with SMF's own `h6xtea.exe`.
pub fn stale(smf_folder: &Path, game_root: &Path) -> Option<Stale> {
    let h6xtea = smf_folder.join("Third-Party").join("h6xtea.exe");
    if !h6xtea.is_file() {
        return None;
    }
    stale_with(smf_folder, game_root, |f| read_build_id(&h6xtea, f))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reads_the_build_id_line() {
        assert_eq!(build_id("[Hitman5]\r\nBuildID=10814523\r\nOther=1\r\n"), Some(10_814_523));
        assert_eq!(build_id("no id here"), None);
    }

    #[test]
    fn only_an_older_clean_copy_is_stale() {
        let tmp = tempfile::tempdir().unwrap();
        let (smf, game) = (tmp.path().join("smf"), tmp.path().join("game"));
        std::fs::create_dir_all(&smf).unwrap();
        std::fs::create_dir_all(game.join("Retail")).unwrap();
        std::fs::write(game.join("Retail/thumbs.dat"), "10814523").unwrap();
        // The stand-in reader takes the BuildID as the file's text.
        let read = |p: &Path| std::fs::read_to_string(p).ok()?.parse().ok();
        assert_eq!(stale_with(&smf, &game, read), None, "no clean copy yet");
        std::fs::write(smf.join("cleanThumbs.dat"), "8337407").unwrap();
        assert_eq!(stale_with(&smf, &game, read), Some(Stale { clean: 8_337_407, game: 10_814_523 }));
        std::fs::write(smf.join("cleanThumbs.dat"), "10814523").unwrap();
        assert_eq!(stale_with(&smf, &game, read), None, "same build");
        std::fs::write(smf.join("cleanThumbs.dat"), "garbage").unwrap();
        assert_eq!(stale_with(&smf, &game, read), None, "unreadable is never stale");
    }

    /// Against SMF's real h6xtea.exe and a real thumbs.dat, when
    /// `VMM_REAL_SMF` names an SMF folder (read only; run by hand).
    #[test]
    #[ignore]
    fn reads_a_real_thumbs_file() {
        let smf = std::path::PathBuf::from(std::env::var("VMM_REAL_SMF").unwrap());
        let id = read_build_id(&smf.join("Third-Party/h6xtea.exe"), &smf.parent().unwrap().join("Retail/thumbs.dat"));
        assert!(id.is_some_and(|i| i > 0), "{id:?}");
    }
}
