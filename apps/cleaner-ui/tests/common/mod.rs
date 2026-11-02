#![allow(dead_code)]

use std::{
    fs,
    path::{Path, PathBuf},
    time::{SystemTime, UNIX_EPOCH},
};

pub struct UiFixture {
    pub root: PathBuf,
    pub tree: PathBuf,
    pub state: PathBuf,
    pub expected_files: u64,
    pub expected_bytes: u64,
    token: String,
}

impl UiFixture {
    pub fn create() -> Self {
        let token = format!(
            "rusten-ui-{}-{}",
            std::process::id(),
            SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        );
        let parent = std::env::temp_dir().join("rusten-ui-fixtures");
        fs::create_dir_all(&parent).unwrap();
        let root = parent.join(&token);
        fs::create_dir(&root).expect("fixture must be new");
        fs::write(root.join(".owner"), token.as_bytes()).unwrap();
        let canonical = fs::canonicalize(&root).unwrap();
        let canonical_text = canonical.to_str().unwrap();
        let root = PathBuf::from(
            canonical_text
                .strip_prefix(r"\\?\")
                .unwrap_or(canonical_text),
        );
        let tree = root.join("tree");
        let state = root.join("state");
        fs::create_dir_all(tree.join("nested")).unwrap();
        fs::create_dir(&state).unwrap();
        for (name, contents) in [
            ("victim.bin", b"restore-me".as_slice()),
            ("changed.bin", b"identity-before".as_slice()),
            ("keep.bin", b"never-change".as_slice()),
            ("nested/readme.txt", b"nested-text".as_slice()),
        ] {
            fs::write(tree.join(name), contents).unwrap();
        }
        Self {
            root,
            tree,
            state,
            expected_files: 4,
            expected_bytes: 10 + 15 + 12 + 11,
            token,
        }
    }

    pub fn assert_owned_path(&self, path: &Path) {
        let root = fs::canonicalize(&self.root).expect("fixture root resolves");
        assert_eq!(root.file_name().unwrap().to_string_lossy(), self.token);
        assert_eq!(
            fs::read(root.join(".owner")).unwrap(),
            self.token.as_bytes()
        );
        let candidate =
            fs::canonicalize(path).expect("target must resolve before destructive work");
        assert_ne!(candidate, root, "target must be a descendant");
        assert!(
            candidate.starts_with(&root),
            "target must belong to fixture"
        );
    }
}

impl Drop for UiFixture {
    fn drop(&mut self) {
        // Validate an owned descendant before removing the uniquely created root.
        self.assert_owned_path(&self.root.join(".owner"));
        fs::remove_dir_all(&self.root).unwrap();
    }
}
