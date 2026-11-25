use slint::SharedString;
#[cfg(unix)]
use std::os::unix::fs::MetadataExt;
use std::{
    fs,
    io::{self, BufReader, BufWriter, Read, Write},
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
        // Reject devices, pipes and directories before opening them.
        if let Ok(metadata) = fs::metadata(&path) {
            validate_file(&metadata)?;
        }
        match fs::File::open(&path) {
            Ok(mut file) => {
                let metadata = file.metadata()?;
                validate_file(&metadata)?;
                let total = metadata.len();
                validate_size(total)?;
                // Do not trust the reported size enough to allocate it all at once.
                let mut bytes = Vec::new();
                let mut chunk = [0; 64 * 1024];
                let mut loaded = 0_u64;
                let mut last_reported = 0_u64;
                loop {
                    let count = file.read(&mut chunk)?;
                    if count == 0 {
                        break;
                    }
                    loaded += count as u64;
                    validate_size(loaded)?;
                    bytes.try_reserve(count).map_err(io::Error::other)?;
                    bytes.extend_from_slice(&chunk[..count]);
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
        let same = self.text.as_ptr() == text.as_ptr() && self.text.len() == text.len();
        (!same && self.text.as_str() != text)
            || (self.path.is_some() && self.original_exists.is_none())
    }

    #[cfg(test)]
    pub fn save(&mut self, path: &Path, text: &str) -> io::Result<()> {
        self.save_snapshot(path, text.into())
    }

    pub fn save_snapshot(&mut self, path: &Path, text: SharedString) -> io::Result<()> {
        // The original path was canonicalized when opened. Do not follow a new
        // symlink substituted at that path while the document was being edited.
        let destination = if self.path.as_deref() == Some(path) {
            path.to_path_buf()
        } else if path.exists() {
            fs::canonicalize(path)?
        } else {
            absolute(path)?
        };
        let same_destination = self.path.as_ref() == Some(&destination);
        let bom = self.bom;
        let crlf = self.crlf;
        atomic_write(
            &destination,
            |writer| write_document(writer, &text, bom, crlf),
            || {
                if same_destination {
                    let unchanged = match self.original_exists {
                        Some(true) => {
                            matches_file(&destination, self.text.as_str(), self.bom, self.crlf)?
                        }
                        _ => !destination.exists(),
                    };
                    if !unchanged {
                        return Err(conflict());
                    }
                }
                Ok(())
            },
        )?;
        self.path = Some(destination);
        self.text = text;
        self.original_exists = Some(true);
        Ok(())
    }
}

fn validate_size(size: u64) -> io::Result<()> {
    if size > i32::MAX as u64 {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            "The file exceeds the editor's signed 32-bit byte-offset limit.",
        ));
    }
    Ok(())
}

fn validate_file(metadata: &fs::Metadata) -> io::Result<()> {
    if !metadata.is_file() {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            "Only regular text files are supported.",
        ));
    }
    Ok(())
}

fn conflict() -> io::Error {
    io::Error::other(
        "The file changed on disk. Use Save as to save a separate copy, or reopen the file.",
    )
}

fn write_document(writer: &mut dyn Write, text: &str, bom: bool, crlf: bool) -> io::Result<()> {
    if bom {
        writer.write_all(&[0xef, 0xbb, 0xbf])?;
    }
    if crlf {
        for line in text.split_inclusive('\n') {
            if let Some(body) = line.strip_suffix('\n') {
                writer.write_all(body.as_bytes())?;
                writer.write_all(b"\r\n")?;
            } else {
                writer.write_all(line.as_bytes())?;
            }
        }
    } else {
        writer.write_all(text.as_bytes())?;
    }
    Ok(())
}

fn matches_file(path: &Path, text: &str, bom: bool, crlf: bool) -> io::Result<bool> {
    let mut file = match fs::File::open(path) {
        Ok(file) => BufReader::with_capacity(64 * 1024, file),
        Err(error) if error.kind() == io::ErrorKind::NotFound => return Ok(false),
        Err(error) => return Err(error),
    };
    let mut actual = [0; 64 * 1024];

    if bom {
        let mut marker = [0; 3];
        match file.read_exact(&mut marker) {
            Ok(()) => {}
            Err(error) if error.kind() == io::ErrorKind::UnexpectedEof => return Ok(false),
            Err(error) => return Err(error),
        }
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

fn matches_bytes(file: &mut impl Read, actual: &mut [u8], expected: &[u8]) -> io::Result<bool> {
    for part in expected.chunks(actual.len()) {
        match file.read_exact(&mut actual[..part.len()]) {
            Ok(()) => {}
            Err(error) if error.kind() == io::ErrorKind::UnexpectedEof => return Ok(false),
            Err(error) => return Err(error),
        }
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

fn destination_metadata(path: &Path) -> io::Result<Option<fs::Metadata>> {
    match fs::symlink_metadata(path) {
        Ok(metadata) => {
            validate_file(&metadata)?;
            Ok(Some(metadata))
        }
        Err(error) if error.kind() == io::ErrorKind::NotFound => Ok(None),
        Err(error) => Err(error),
    }
}

fn same_metadata(before: &fs::Metadata, after: &fs::Metadata) -> bool {
    #[cfg(unix)]
    if (
        before.dev(),
        before.ino(),
        before.ctime(),
        before.ctime_nsec(),
        before.nlink(),
    ) != (
        after.dev(),
        after.ino(),
        after.ctime(),
        after.ctime_nsec(),
        after.nlink(),
    ) {
        return false;
    }
    before.len() == after.len() && before.modified().ok() == after.modified().ok()
}

fn atomic_write(
    path: &Path,
    write: impl FnOnce(&mut dyn Write) -> io::Result<()>,
    verify: impl FnOnce() -> io::Result<()>,
) -> io::Result<()> {
    let parent = path
        .parent()
        .ok_or_else(|| io::Error::other("No parent directory"))?;
    let metadata = destination_metadata(path)?;
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
    #[cfg(windows)]
    let acl = if metadata.is_some() {
        // Apply permissions before writing any document contents to the temporary file.
        crate::windows_acl::PreservedAcl::prepare(path, temp.path())?
    } else {
        None
    };
    {
        let mut writer = BufWriter::new(temp.as_file_mut());
        write(&mut writer)?;
        writer.flush()?;
    }
    if let Some(meta) = &metadata {
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
        #[cfg(target_os = "linux")]
        {
            use std::os::unix::fs::OpenOptionsExt;
            let source = fs::OpenOptions::new()
                .read(true)
                .custom_flags(libc::O_NOFOLLOW | libc::O_NONBLOCK)
                .open(path)?;
            if !same_metadata(meta, &source.metadata()?) {
                return Err(conflict());
            }
            crate::xattrs::copy(&source, file)?;
        }
    }
    temp.as_file().sync_all()?;
    // Keep verification close to the rename, after the potentially slow write.
    // Recheck file identity as well as contents. A portable rename still cannot
    // provide compare-and-swap semantics for an existing destination.
    verify()?;
    #[cfg(windows)]
    if let Some(acl) = acl {
        acl.verify()?;
    }
    match (&metadata, destination_metadata(path)?) {
        (Some(before), Some(after)) if same_metadata(before, &after) => {
            temp.persist(path).map_err(|e| e.error)?;
        }
        (None, None) => {
            // Unlike a normal rename, this cannot overwrite a newly created file.
            temp.persist_noclobber(path).map_err(|e| e.error)?;
        }
        _ => return Err(conflict()),
    }
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
    fn refuses_destination_created_during_save() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("note");
        let result = atomic_write(
            &path,
            |writer| writer.write_all(b"mine"),
            || fs::write(&path, b"external"),
        );
        assert!(result.is_err());
        assert_eq!(fs::read(&path).unwrap(), b"external");
    }

    #[test]
    fn save_snapshot_reuses_shared_text_and_detects_truncation() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("note");
        let mut doc = Document::open(&path).unwrap();
        let text: SharedString = "first line\nsecond line\n".into();
        doc.save_snapshot(&path, text.clone()).unwrap();
        assert_eq!(doc.text.as_ptr(), text.as_ptr());
        assert!(!doc.dirty(&text));
        fs::write(&path, b"first").unwrap();
        assert!(doc.save(&path, "mine").is_err());
        assert_eq!(fs::read(&path).unwrap(), b"first");
    }

    #[test]
    #[cfg(unix)]
    fn rejects_symlink_substituted_for_original_path() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("note");
        let other = dir.path().join("other");
        fs::write(&path, b"original").unwrap();
        fs::write(&other, b"other").unwrap();
        let mut doc = Document::open(&path).unwrap();
        fs::remove_file(&path).unwrap();
        symlink(&other, &path).unwrap();
        let destination = doc.path.clone().unwrap();
        assert!(doc.save(&destination, "mine").is_err());
        assert_eq!(fs::read(&other).unwrap(), b"other");
    }

    #[test]
    fn rejects_directories_and_oversized_file_lengths() {
        let dir = tempfile::tempdir().unwrap();
        assert!(Document::open(dir.path()).is_err());
        assert!(validate_size(i32::MAX as u64).is_ok());
        assert!(validate_size(i32::MAX as u64 + 1).is_err());
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
