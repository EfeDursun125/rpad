use slint::SharedString;
#[cfg(unix)]
use std::os::unix::fs::MetadataExt;
use std::{
    fs,
    io::{self, Read, Write},
    path::{Path, PathBuf},
};

#[derive(Clone, Default)]
pub struct Document {
    pub path: Option<PathBuf>,
    pub text: SharedString,
    original_exists: Option<bool>,
    pub crlf: bool,
    pub bom: bool,
}

impl Document {
    pub fn open(path: &Path) -> io::Result<Self> {
        Self::open_with_progress(path, |_, _| {})
    }

    pub fn open_with_progress(path: &Path, mut progress: impl FnMut(u64, u64)) -> io::Result<Self> {
        let path = if path.exists() {
            fs::canonicalize(path)?
        } else {
            absolute(path)?
        };
        match fs::File::open(&path) {
            Ok(mut file) => {
                let total = file.metadata()?.len();
                let mut bytes = Vec::with_capacity(total.try_into().unwrap_or(0));
                let mut chunk = [0; 64 * 1024];
                let mut loaded = 0_u64;
                let mut last_reported = 0_u64;
                loop {
                    let count = file.read(&mut chunk)?;
                    if count == 0 {
                        break;
                    }
                    bytes.extend_from_slice(&chunk[..count]);
                    loaded += count as u64;
                    if loaded == total || loaded - last_reported >= 8 * 1024 * 1024 {
                        progress(loaded, total);
                        last_reported = loaded;
                    }
                }
                let bom = bytes.starts_with(&[0xef, 0xbb, 0xbf]);
                if bom {
                    bytes.copy_within(3.., 0);
                    bytes.truncate(bytes.len() - 3);
                }
                let source = std::str::from_utf8(&bytes).map_err(|_| {
                    io::Error::new(
                        io::ErrorKind::InvalidData,
                        "This file is not UTF-8. Convert its encoding before opening it in rpad.",
                    )
                })?;
                if source.contains('\0') {
                    return Err(io::Error::new(
                        io::ErrorKind::InvalidData,
                        "This looks like a binary file.",
                    ));
                }
                let has_crlf = source.contains("\r\n");
                // Mixed line endings stay verbatim to avoid silently rewriting a file.
                let crlf = has_crlf
                    && source
                        .match_indices('\n')
                        .all(|(offset, _)| offset > 0 && source.as_bytes()[offset - 1] == b'\r');
                if crlf {
                    let mut read = 0;
                    let mut write = 0;
                    while read < bytes.len() {
                        if bytes[read] == b'\r' && bytes.get(read + 1) == Some(&b'\n') {
                            read += 1;
                        }
                        bytes[write] = bytes[read];
                        read += 1;
                        write += 1;
                    }
                    bytes.truncate(write);
                }
                let text =
                    String::from_utf8(bytes).expect("UTF-8 was validated before normalization");
                Ok(Self {
                    path: Some(path),
                    text: text.into(),
                    original_exists: Some(true),
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
        self.text.as_str() != text || (self.path.is_some() && self.original_exists.is_none())
    }

    pub fn save(&mut self, path: &Path, text: &str) -> io::Result<()> {
        let destination = if path.exists() {
            fs::canonicalize(path)?
        } else {
            absolute(path)?
        };
        if self.path.as_ref() == Some(&destination) {
            let unchanged = match self.original_exists {
                Some(true) => matches_file(&destination, self.text.as_str(), self.bom, self.crlf)?,
                Some(false) => !destination.exists(),
                None => !destination.exists(),
            };
            if !unchanged {
                return Err(io::Error::other("The file changed on disk. Use Save as to save a separate copy, or reopen the file."));
            }
        }
        let extra_newlines = if self.crlf {
            text.bytes().filter(|byte| *byte == b'\n').count()
        } else {
            0
        };
        let mut bytes =
            Vec::with_capacity(text.len() + extra_newlines + if self.bom { 3 } else { 0 });
        if self.bom {
            bytes.extend_from_slice(&[0xef, 0xbb, 0xbf]);
        }
        if self.crlf {
            for (index, part) in text.split('\n').enumerate() {
                if index > 0 {
                    bytes.extend_from_slice(b"\r\n");
                }
                bytes.extend_from_slice(part.as_bytes());
            }
        } else {
            bytes.extend_from_slice(text.as_bytes());
        }
        atomic_write(&destination, &bytes)?;
        self.path = Some(destination);
        self.text = text.into();
        self.original_exists = Some(true);
        Ok(())
    }
}

fn matches_file(path: &Path, text: &str, bom: bool, crlf: bool) -> io::Result<bool> {
    let mut file = match fs::File::open(path) {
        Ok(file) => file,
        Err(error) if error.kind() == io::ErrorKind::NotFound => return Ok(false),
        Err(error) => return Err(error),
    };
    let mut actual = [0; 64 * 1024];

    if bom {
        let mut marker = [0; 3];
        file.read_exact(&mut marker)?;
        if marker != [0xef, 0xbb, 0xbf] {
            return Ok(false);
        }
    }

    if crlf {
        for line in text.split_inclusive('\n') {
            let (body, ending) = line
                .strip_suffix('\n')
                .map_or((line, false), |body| (body, true));
            if !matches_bytes(&mut file, &mut actual, body.as_bytes())? {
                return Ok(false);
            }
            if ending && !matches_bytes(&mut file, &mut actual, b"\r\n")? {
                return Ok(false);
            }
        }
    } else if !matches_bytes(&mut file, &mut actual, text.as_bytes())? {
        return Ok(false);
    }

    let mut extra = [0; 1];
    Ok(file.read(&mut extra)? == 0)
}

fn matches_bytes(file: &mut fs::File, actual: &mut [u8], expected: &[u8]) -> io::Result<bool> {
    for part in expected.chunks(actual.len()) {
        file.read_exact(&mut actual[..part.len()])?;
        if &actual[..part.len()] != part {
            return Ok(false);
        }
    }
    Ok(true)
}

fn absolute(path: &Path) -> io::Result<PathBuf> {
    let path = std::path::absolute(path)?;
    let parent = path
        .parent()
        .ok_or_else(|| io::Error::other("No parent directory"))?;
    let name = path
        .file_name()
        .ok_or_else(|| io::Error::other("No file name"))?;

    // Match canonical paths even after a file is deleted (including Windows' prefix).
    Ok(fs::canonicalize(parent)?.join(name))
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
    fn rejects_file_created_externally_before_first_save() {
        let dir = tempfile::tempdir().unwrap();
        fs::create_dir(dir.path().join("sub")).unwrap();
        let path = dir.path().join("sub").join("..").join("new note");
        let mut doc = Document::open(&path).unwrap();

        fs::write(&path, "external").unwrap();

        assert!(doc.save(&path, "mine").is_err());
        assert_eq!(fs::read_to_string(&path).unwrap(), "external");
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
