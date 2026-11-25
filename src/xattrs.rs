//! Preserve Linux extended attributes, including POSIX ACLs, during atomic saves.
use std::{
    ffi::{CStr, CString},
    fs::File,
    io,
    os::fd::AsRawFd,
};

fn read_value(mut get: impl FnMut(*mut libc::c_void, usize) -> isize) -> io::Result<Vec<u8>> {
    for _ in 0..4 {
        let size = get(std::ptr::null_mut(), 0);
        if size < 0 {
            return Err(io::Error::last_os_error());
        }
        if size == 0 {
            return Ok(Vec::new());
        }
        let mut buffer = vec![0; size as usize];
        let read = get(buffer.as_mut_ptr().cast(), buffer.len());
        if read >= 0 && read as usize <= buffer.len() {
            buffer.truncate(read as usize);
            return Ok(buffer);
        }
        let error = io::Error::last_os_error();
        if read < 0 && error.raw_os_error() != Some(libc::ERANGE) {
            return Err(error);
        }
    }
    Err(io::Error::other("Extended attributes changed while saving"))
}

fn names(file: &File) -> io::Result<Vec<CString>> {
    let result =
        read_value(|ptr, size| unsafe { libc::flistxattr(file.as_raw_fd(), ptr.cast(), size) });
    let bytes = match result {
        Ok(bytes) => bytes,
        Err(error) if error.raw_os_error() == Some(libc::EOPNOTSUPP) => return Ok(Vec::new()),
        Err(error) => return Err(error),
    };
    if !bytes.is_empty() && bytes.last() != Some(&0) {
        return Err(io::Error::other("Invalid extended attribute list"));
    }
    bytes
        .split(|byte| *byte == 0)
        .filter(|name| !name.is_empty())
        .map(|name| CString::new(name).map_err(io::Error::other))
        .collect()
}

fn value(file: &File, name: &CStr) -> io::Result<Vec<u8>> {
    read_value(|ptr, size| unsafe { libc::fgetxattr(file.as_raw_fd(), name.as_ptr(), ptr, size) })
}

fn set(file: &File, name: &CStr, bytes: &[u8]) -> io::Result<()> {
    let result = unsafe {
        libc::fsetxattr(
            file.as_raw_fd(),
            name.as_ptr(),
            bytes.as_ptr().cast(),
            bytes.len(),
            0,
        )
    };
    if result == 0 {
        Ok(())
    } else {
        Err(io::Error::last_os_error())
    }
}

pub fn copy(source: &File, destination: &File) -> io::Result<()> {
    let source_names = names(source)?;
    // A temporary file can inherit a directory's default ACL or security label.
    // Remove inherited attributes absent from the original rather than widening access.
    for name in names(destination)? {
        if !source_names.contains(&name) {
            let result = unsafe { libc::fremovexattr(destination.as_raw_fd(), name.as_ptr()) };
            if result != 0 {
                return Err(io::Error::last_os_error());
            }
        }
    }
    for name in source_names {
        let bytes = value(source, &name)?;
        if value(destination, &name).ok().as_ref() != Some(&bytes) {
            set(destination, &name, &bytes).map_err(|error| {
                io::Error::other(format!(
                    "Could not preserve extended attribute {}: {error}",
                    name.to_string_lossy()
                ))
            })?;
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn save_preserves_user_attributes() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("note");
        std::fs::write(&path, "original").unwrap();
        let name = c"user.rpad-test";
        let file = File::open(&path).unwrap();
        if let Err(error) = set(&file, name, b"metadata") {
            if error.raw_os_error() == Some(libc::EOPNOTSUPP) {
                return;
            }
            panic!("{error}");
        }
        let mut document = crate::document::Document::open(&path).unwrap();
        document.save(&path, "changed").unwrap();
        assert_eq!(
            value(&File::open(&path).unwrap(), name).unwrap(),
            b"metadata"
        );
    }
}
