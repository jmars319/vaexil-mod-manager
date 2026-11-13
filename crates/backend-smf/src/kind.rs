//! Works out what a mod mostly changes from the files it ships. SMF manifests
//! have no type field, so this is a best guess for grouping the Mods view and
//! never affects what SMF loads.

use std::path::Path;

use vmm_core::backend::ModKind;

/// Entity files patch almost anything (outfits, lighting, AI), so they count
/// for less than files that say plainly what they are.
const ENTITY_WEIGHT: f64 = 0.25;
/// A manifest that brings its own text counts as this many text files.
const LOCALISATION_WEIGHT: f64 = 2.0;

/// File-name endings and the kind each one points to. Longer endings come
/// first so `.entity.patch.json` is not read as plain JSON.
const ENDINGS: &[(&str, ModKind, f64)] = &[
    (".entity.patch.json", ModKind::Gameplay, ENTITY_WEIGHT),
    (".entity.json", ModKind::Gameplay, ENTITY_WEIGHT),
    (".repository.json", ModKind::Gameplay, 1.0),
    (".unlockables.json", ModKind::Gameplay, 1.0),
    (".contract.json", ModKind::Gameplay, 1.0),
    (".material.json", ModKind::ModelsTextures, 1.0),
    (".dlge.json", ModKind::Dialogue, 1.0),
    (".sdef.json", ModKind::Sound, 1.0),
    (".locr.json", ModKind::Text, 1.0),
    (".rtlv.json", ModKind::Text, 1.0),
    (".temp", ModKind::Gameplay, ENTITY_WEIGHT),
    (".tblu", ModKind::Gameplay, ENTITY_WEIGHT),
    (".prim", ModKind::ModelsTextures, 1.0),
    (".borg", ModKind::ModelsTextures, 1.0),
    (".text", ModKind::ModelsTextures, 1.0),
    (".texd", ModKind::ModelsTextures, 1.0),
    (".tga", ModKind::ModelsTextures, 1.0),
    (".dds", ModKind::ModelsTextures, 1.0),
    (".mati", ModKind::ModelsTextures, 1.0),
    (".matt", ModKind::ModelsTextures, 1.0),
    (".matb", ModKind::ModelsTextures, 1.0),
    (".mate", ModKind::ModelsTextures, 1.0),
    (".dlge", ModKind::Dialogue, 1.0),
    (".fxas", ModKind::Dialogue, 1.0),
    (".wwes", ModKind::Dialogue, 1.0),
    (".wem", ModKind::Sound, 1.0),
    (".wwev", ModKind::Sound, 1.0),
    (".wwem", ModKind::Sound, 1.0),
    (".wbnk", ModKind::Sound, 1.0),
    (".sdef", ModKind::Sound, 1.0),
    (".gfxf", ModKind::Ui, 1.0),
    (".swff", ModKind::Ui, 1.0),
    (".locr", ModKind::Text, 1.0),
    (".clng", ModKind::Text, 1.0),
    (".ditl", ModKind::Text, 1.0),
];

/// The kind with the most weight among the mod's files. A mod with nothing
/// recognisable, such as an RPKG-only mod, is `Other`.
pub fn classify(folder: &Path, has_localisation: bool) -> ModKind {
    let mut scores = [0.0f64; ModKind::ALL.len()];
    if has_localisation {
        scores[index(ModKind::Text)] += LOCALISATION_WEIGHT;
    }
    let mut stack = vec![folder.to_path_buf()];
    while let Some(dir) = stack.pop() {
        let Ok(entries) = std::fs::read_dir(&dir) else { continue };
        for entry in entries.flatten() {
            let Ok(file_type) = entry.file_type() else { continue };
            if file_type.is_dir() {
                stack.push(entry.path());
            } else if let Some((kind, weight)) = kind_of(&entry.file_name().to_string_lossy()) {
                scores[index(kind)] += weight;
            }
        }
    }
    let mut best = ModKind::Other;
    let mut best_score = 0.0;
    // Ties go to the kind listed first.
    for kind in ModKind::ALL {
        if scores[index(kind)] > best_score {
            best = kind;
            best_score = scores[index(kind)];
        }
    }
    best
}

fn kind_of(file_name: &str) -> Option<(ModKind, f64)> {
    let lower = file_name.to_ascii_lowercase();
    ENDINGS.iter().find(|(ending, _, _)| lower.ends_with(ending)).map(|&(_, kind, weight)| (kind, weight))
}

fn index(kind: ModKind) -> usize {
    ModKind::ALL.iter().position(|k| *k == kind).expect("every kind is in ALL")
}

#[cfg(test)]
mod tests {
    use super::*;

    fn mod_with(files: &[&str]) -> tempfile::TempDir {
        let tmp = tempfile::tempdir().unwrap();
        for f in files {
            let p = tmp.path().join(f);
            std::fs::create_dir_all(p.parent().unwrap()).unwrap();
            std::fs::write(p, b"").unwrap();
        }
        tmp
    }

    #[test]
    fn reads_the_kind_from_the_files() {
        // Shaped like mods on JASON-PC, 2026-11-12.
        let accents = mod_with(&["c/chunk0/a.WWES", "c/chunk0/b.wwes", "c/chunk0/c.fxas", "c/chunk0/x.sdef"]);
        assert_eq!(classify(accents.path(), false), ModKind::Dialogue);
        let pistols = mod_with(&["content/chunk0/1.wem", "content/chunk0/2.wem", "manifest.json"]);
        assert_eq!(classify(pistols.path(), false), ModKind::Sound);
        let readable = mod_with(&["c/a.GFXF", "c/b.gfxf", "c/a.GFXF.meta"]);
        assert_eq!(classify(readable.path(), true), ModKind::Ui);
        let ai = mod_with(&["c/a.repository.json", "manifest.json"]);
        assert_eq!(classify(ai.path(), false), ModKind::Gameplay);
        let meshes = mod_with(&["a.temp", "b.tblu", "c.prim", "d.prim", "e.TEXT"]);
        assert_eq!(classify(meshes.path(), false), ModKind::ModelsTextures);
    }

    #[test]
    fn text_comes_from_the_manifest_when_files_say_nothing() {
        let renaming = mod_with(&["manifest.json", "LICENSE"]);
        assert_eq!(classify(renaming.path(), true), ModKind::Text);
    }

    #[test]
    fn unrecognised_files_are_other() {
        let rpkg = mod_with(&["chunk0patch300.rpkg", "readme.txt", "preview.jpg"]);
        assert_eq!(classify(rpkg.path(), false), ModKind::Other);
        assert_eq!(classify(Path::new("does-not-exist"), false), ModKind::Other);
    }

    #[test]
    fn entity_files_count_for_less() {
        // Four entity patches (1.0) lose to two textures.
        let glasses = mod_with(&[
            "a.entity.patch.json",
            "b.entity.patch.json",
            "c.entity.json",
            "d.entity.json",
            "e.tga",
            "f.tga",
        ]);
        assert_eq!(classify(glasses.path(), false), ModKind::ModelsTextures);
    }
}
