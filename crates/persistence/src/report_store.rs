use std::{
    fs, io,
    path::{Path, PathBuf},
};

pub struct ReportStore {
    root: PathBuf,
}

impl ReportStore {
    pub fn open(root: impl Into<PathBuf>) -> io::Result<Self> {
        let root = root.into();
        fs::create_dir_all(&root)?;
        Ok(Self { root })
    }

    pub fn write(&self, name: &str, content: &[u8]) -> io::Result<PathBuf> {
        let path = self.root.join(name);
        if path.file_name().and_then(|name| name.to_str()) != Some(name) {
            return Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                "report name must be a file name",
            ));
        }
        fs::write(&path, content)?;
        Ok(path)
    }

    pub fn root(&self) -> &Path {
        &self.root
    }
}
