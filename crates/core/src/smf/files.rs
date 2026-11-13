//! Which game files a mod replaces wholesale, for the "overridden by"
//! markers. SMF mods ship raw replacements named by the resource's hash, such
//! as `00B9217B4DB441A0.TEMP`. When two enabled mods ship the same one, the
//! later mod's file is used. Patch files (`.entity.patch.json`,
//! `.repository.json` and the like) merge instead, so they never count, and
//! neither do the `.meta` files that travel with raw files. Files named any
//! other way (QuickEntity `.entity.json`, for instance, keeps its hash inside)
//! are not compared, so the markers can miss an override but never invent one.

use std::path::Path;

/// The resource keys (`HASH.TYPE`, upper case) of every hash-named raw file
/// under `dir`.
pub fn raw_resources(dir: &Path) -> Vec<String> {
    let mut out = Vec::new();
    let mut stack = vec![dir.to_path_buf()];
    while let Some(d) = stack.pop() {
        let Ok(entries) = std::fs::read_dir(&d) else { continue };
        for e in entries.flatten() {
            let Ok(t) = e.file_type() else { continue };
            if t.is_dir() {
                stack.push(e.path());
            } else if let Some(key) = resource_key(&e.file_name().to_string_lossy()) {
                out.push(key);
            }
        }
    }
    out.sort();
    out.dedup();
    out
}

fn resource_key(file_name: &str) -> Option<String> {
    let (stem, rest) = file_name.split_once('.')?;
    if stem.len() != 16 || !stem.bytes().all(|b| b.is_ascii_hexdigit()) || rest.is_empty() {
        return None;
    }
    let rest = rest.to_ascii_uppercase();
    if rest.contains('.') || rest.ends_with("META") {
        return None;
    }
    Some(format!("{}.{rest}", stem.to_ascii_uppercase()))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn only_hash_named_raw_files_count() {
        assert_eq!(resource_key("00b9217b4db441a0.temp").as_deref(), Some("00B9217B4DB441A0.TEMP"));
        assert_eq!(resource_key("00B9217B4DB441A0.PRIM").as_deref(), Some("00B9217B4DB441A0.PRIM"));
        for skip in [
            "00B9217B4DB441A0.TEMP.meta",
            "00B9217B4DB441A0.TEMP.meta.json",
            "00B9217B4DB441A0.entity.patch.json",
            "suit.entity.json",
            "00B9217B4DB441A.TEMP",
            "manifest.json",
            "ZZB9217B4DB441A0.TEMP",
        ] {
            assert_eq!(resource_key(skip), None, "{skip}");
        }
    }

    #[test]
    fn walks_every_chunk_folder() {
        let tmp = tempfile::tempdir().unwrap();
        for f in
            ["chunk0/00AA00AA00AA00AA.TEMP", "chunk1/00aa00aa00aa00aa.temp", "chunk1/00BB00BB00BB00BB.PRIM", "x.txt"]
        {
            let p = tmp.path().join(f);
            std::fs::create_dir_all(p.parent().unwrap()).unwrap();
            std::fs::write(p, b"").unwrap();
        }
        assert_eq!(raw_resources(tmp.path()), ["00AA00AA00AA00AA.TEMP", "00BB00BB00BB00BB.PRIM"]);
    }
}
