#[cfg(unix)]
use std::os::unix::fs::MetadataExt;
use std::{
    fs,
    io::{self, Write},
    path::{Path, PathBuf},
};

#[derive(Clone, Default)]
pub struct Document {
    pub path: Option<PathBuf>,
    pub text: String,
    original: Option<Vec<u8>>,
    pub crlf: bool,
    pub bom: bool,
}

impl Document {
    pub fn open(path: &Path) -> io::Result<Self> {
        let path = if path.exists() {
            fs::canonicalize(path)?
        } else {
            absolute(path)?
        };
        match fs::read(&path) {
            Ok(bytes) => {
                let bom = bytes.starts_with(&[0xef, 0xbb, 0xbf]);
                let data = if bom { &bytes[3..] } else { &bytes };
                let text = std::str::from_utf8(data).map_err(|_| {
                    io::Error::new(
                        io::ErrorKind::InvalidData,
                        "This file is not UTF-8. Convert its encoding before opening it in rpad.",
                    )
                })?;
                if text.contains('\0') {
                    return Err(io::Error::new(
                        io::ErrorKind::InvalidData,
                        "This looks like a binary file.",
                    ));
                }
                let crlf = text.contains("\r\n");
                // Mixed line endings stay verbatim to avoid silently rewriting a file.
                let crlf = crlf && !text.replace("\r\n", "").contains('\n');
                let text = if crlf {
                    text.replace("\r\n", "\n")
                } else {
                    text.to_owned()
                };
                Ok(Self {
                    path: Some(path),
                    text,
                    original: Some(bytes),
                    crlf,
                    bom,
                })
            }
            Err(error)
                if error.kind() == io::ErrorKind::NotFound
                    && path.parent().is_some_and(Path::is_dir) =>
            {
                Ok(Self {
                    path: Some(path),
                    ..Self::default()
                })
            }
            Err(error) => Err(error),
        }
    }

    pub fn dirty(&self, text: &str) -> bool {
        self.text != text || (self.path.is_some() && self.original.is_none())
    }

    pub fn save(&mut self, path: &Path, text: &str) -> io::Result<()> {
        let destination = if path.exists() {
            fs::canonicalize(path)?
        } else {
            absolute(path)?
        };
        if self.path.as_ref() == Some(&destination) {
            let current = match fs::read(&destination) {
                Ok(bytes) => Some(bytes),
                Err(e) if e.kind() == io::ErrorKind::NotFound => None,
                Err(e) => return Err(e),
            };
            if current != self.original {
                return Err(io::Error::other("The file changed on disk. Use Save as to save a separate copy, or reopen the file."));
            }
        }
        let mut bytes = Vec::new();
        if self.bom {
            bytes.extend_from_slice(&[0xef, 0xbb, 0xbf]);
        }
        let encoded = if self.crlf {
            text.replace('\n', "\r\n")
        } else {
            text.to_owned()
        };
        bytes.extend_from_slice(encoded.as_bytes());
        atomic_write(&destination, &bytes)?;
        self.path = Some(destination);
        self.text = text.to_owned();
        self.original = Some(bytes);
        Ok(())
    }
}

fn absolute(path: &Path) -> io::Result<PathBuf> {
    if path.is_absolute() {
        Ok(path.to_owned())
    } else {
        Ok(std::env::current_dir()?.join(path))
    }
}

fn atomic_write(path: &Path, bytes: &[u8]) -> io::Result<()> {
    let parent = path
        .parent()
        .ok_or_else(|| io::Error::other("No parent directory"))?;
    let metadata = fs::metadata(path).ok();
    if let Some(_meta) = &metadata {
        #[cfg(unix)]
        if _meta.nlink() > 1 {
            return Err(io::Error::other(
                "This file has hard links. Use Save as to avoid breaking its links.",
            ));
        }
        // Respect file permissions even when its containing directory is writable.
        fs::OpenOptions::new().write(true).open(path)?;
    }
    let mut temp = tempfile::NamedTempFile::new_in(parent)?;
    temp.write_all(bytes)?;
    if let Some(meta) = metadata {
        let file = temp.as_file();
        // Preserve ownership when an administrator edits another user's file.
        #[cfg(unix)]
        if file.metadata()?.uid() != meta.uid() || file.metadata()?.gid() != meta.gid() {
            use std::os::fd::AsRawFd;
            if unsafe { libc::fchown(file.as_raw_fd(), meta.uid(), meta.gid()) } != 0 {
                return Err(io::Error::last_os_error());
            }
        }
        file.set_permissions(meta.permissions())?;
    }
    temp.as_file().sync_all()?;
    temp.persist(path).map_err(|e| e.error)?;
    #[cfg(unix)]
    fs::File::open(parent)?.sync_all()?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    #[cfg(unix)]
    use std::os::unix::fs::{symlink, PermissionsExt};
    #[test]
    #[cfg(unix)]
    fn preserves_bom_crlf_permissions_and_symlink() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("config");
        fs::write(&path, b"\xef\xbb\xbf[config]\r\nsize=4\r\n").unwrap();
        fs::set_permissions(&path, fs::Permissions::from_mode(0o640)).unwrap();
        let link = dir.path().join("link");
        symlink(&path, &link).unwrap();
        let mut doc = Document::open(&link).unwrap();
        assert_eq!(doc.text, "[config]\nsize=4\n");
        doc.save(&link, "[config]\nsize=8\n").unwrap();
        assert_eq!(
            fs::read(&path).unwrap(),
            b"\xef\xbb\xbf[config]\r\nsize=8\r\n"
        );
        assert!(link.is_symlink());
        assert_eq!(
            fs::metadata(path).unwrap().permissions().mode() & 0o777,
            0o640
        );
    }
    #[test]
    fn rejects_external_change_and_deleted_file() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("note");
        fs::write(&path, "original").unwrap();
        let mut doc = Document::open(&path).unwrap();
        fs::write(&path, "external").unwrap();
        assert!(doc.save(&path, "mine").is_err());
        assert_eq!(fs::read_to_string(&path).unwrap(), "external");
        fs::remove_file(&path).unwrap();
        assert!(doc.save(&path, "mine").is_err());
    }
    #[test]
    fn new_unicode_file_and_dirty_state() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("new file");
        let mut doc = Document::open(&path).unwrap();
        assert!(doc.dirty(""));
        doc.save(&path, "Türkçe 🦀\n").unwrap();
        assert!(!doc.dirty("Türkçe 🦀\n"));
        assert_eq!(Document::open(&path).unwrap().text, "Türkçe 🦀\n");
    }
    #[test]
    fn rejects_non_utf8_and_preserves_mixed_endings() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("file");
        fs::write(&path, [0xff, 0xfe]).unwrap();
        assert!(Document::open(&path).is_err());
        fs::write(&path, b"a\r\nb\n").unwrap();
        let mut doc = Document::open(&path).unwrap();
        assert!(!doc.crlf);
        let text = doc.text.clone();
        doc.save(&path, &text).unwrap();
        assert_eq!(fs::read(&path).unwrap(), b"a\r\nb\n");
    }
}
