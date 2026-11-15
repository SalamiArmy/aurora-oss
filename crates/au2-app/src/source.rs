//! Where levels come from. Desktop reads `levels/` from the filesystem; the Android front end implements the same
//! trait on top of the APK's asset manager (there is no directory layout inside an APK).

use au2_map::{load_index, load_level, parse_index, parse_map, LevelEntry, Map};
use std::path::PathBuf;

pub trait LevelSource {
    fn index(&self) -> Result<Vec<LevelEntry>, String>;
    fn level(&self, entry: &LevelEntry) -> Result<Map, String>;
}

/// `<root>/levels/` on disk (the desktop behaviour).
pub struct FsSource {
    pub root: PathBuf,
}

impl LevelSource for FsSource {
    fn index(&self) -> Result<Vec<LevelEntry>, String> {
        load_index(&self.root.join("levels")).map_err(|e| e.to_string())
    }
    fn level(&self, entry: &LevelEntry) -> Result<Map, String> {
        load_level(&self.root.join("levels"), entry).map_err(|e| e.to_string())
    }
}

/// Levels read through a "give me the text of this relative path" function (`levels.txt`, `<stem>.au2`). The Android
/// asset manager is wrapped in one of these.
pub struct TextSource<F: Fn(&str) -> Result<String, String>> {
    pub read: F,
}

impl<F: Fn(&str) -> Result<String, String>> LevelSource for TextSource<F> {
    fn index(&self) -> Result<Vec<LevelEntry>, String> {
        let text = (self.read)("levels/levels.txt")?;
        parse_index(&text).map_err(|e| e.with_file("levels/levels.txt").to_string())
    }
    fn level(&self, entry: &LevelEntry) -> Result<Map, String> {
        let name = format!("levels/{}.au2", entry.stem);
        let text = (self.read)(&name)?;
        parse_map(&text).map_err(|e| e.with_file(name).to_string())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn text_source_loads_every_shipped_level_like_the_filesystem_does() {
        let root = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../..");
        let src = TextSource {
            read: |rel: &str| std::fs::read_to_string(root.join(rel)).map_err(|e| format!("{rel}: {e}")),
        };
        let fs = FsSource { root: root.clone() };
        let a = src.index().unwrap();
        assert_eq!(a, fs.index().unwrap());
        assert!(a.len() >= 30);
        for e in &a {
            src.level(e).unwrap_or_else(|err| panic!("{}: {err}", e.stem));
        }
    }
}
