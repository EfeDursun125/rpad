//! Preserve Windows discretionary ACLs without requiring elevation or extra crates.
use std::{
    ffi::c_void,
    fs::{File, OpenOptions},
    io,
    os::windows::{fs::OpenOptionsExt, io::AsRawHandle},
    path::Path,
    ptr,
};

type Handle = *mut c_void;
type Acl = *mut c_void;

const SE_FILE_OBJECT: u32 = 1;
const DACL_SECURITY_INFORMATION: u32 = 0x0000_0004;
const PROTECTED_DACL_SECURITY_INFORMATION: u32 = 0x8000_0000;
const UNPROTECTED_DACL_SECURITY_INFORMATION: u32 = 0x2000_0000;
const SE_DACL_PRESENT: u16 = 0x0004;
const SE_DACL_PROTECTED: u16 = 0x1000;
const READ_CONTROL: u32 = 0x0002_0000;
const WRITE_DAC: u32 = 0x0004_0000;
const FILE_READ_ATTRIBUTES: u32 = 0x80;
const FILE_FLAG_OPEN_REPARSE_POINT: u32 = 0x0020_0000;
const FILE_PERSISTENT_ACLS: u32 = 0x0000_0008;
const ACL_SIZE_INFORMATION_CLASS: u32 = 2;

#[repr(C)]
#[derive(Default)]
struct AclSizeInformation {
    ace_count: u32,
    bytes_in_use: u32,
    bytes_free: u32,
}

#[link(name = "advapi32")]
extern "system" {
    fn GetSecurityInfo(
        handle: Handle,
        object_type: u32,
        security_info: u32,
        owner: *mut *mut c_void,
        group: *mut *mut c_void,
        dacl: *mut Acl,
        sacl: *mut Acl,
        descriptor: *mut *mut c_void,
    ) -> u32;
    fn SetSecurityInfo(
        handle: Handle,
        object_type: u32,
        security_info: u32,
        owner: *mut c_void,
        group: *mut c_void,
        dacl: Acl,
        sacl: Acl,
    ) -> u32;
    fn GetSecurityDescriptorControl(
        descriptor: *const c_void,
        control: *mut u16,
        revision: *mut u32,
    ) -> i32;
    fn GetAclInformation(acl: Acl, info: *mut c_void, len: u32, class: u32) -> i32;
    fn GetAce(acl: Acl, index: u32, ace: *mut *mut c_void) -> i32;
}

#[link(name = "kernel32")]
extern "system" {
    fn LocalFree(memory: *mut c_void) -> *mut c_void;
    fn GetVolumeInformationByHandleW(
        file: Handle,
        volume_name: *mut u16,
        volume_name_len: u32,
        serial: *mut u32,
        max_component_len: *mut u32,
        flags: *mut u32,
        filesystem_name: *mut u16,
        filesystem_name_len: u32,
    ) -> i32;
}

fn status(code: u32) -> io::Result<()> {
    if code == 0 {
        Ok(())
    } else {
        Err(io::Error::from_raw_os_error(code as i32))
    }
}

fn success(result: i32) -> io::Result<()> {
    if result != 0 {
        Ok(())
    } else {
        Err(io::Error::last_os_error())
    }
}

fn open_security(path: &Path, access: u32) -> io::Result<File> {
    let file = OpenOptions::new()
        .read(true)
        .access_mode(access | FILE_READ_ATTRIBUTES)
        .custom_flags(FILE_FLAG_OPEN_REPARSE_POINT)
        .open(path)?;
    if !file.metadata()?.is_file() || file.metadata()?.file_type().is_symlink() {
        return Err(io::Error::other("Not a regular file for ACL preservation"));
    }
    Ok(file)
}

fn supports_acls(file: &File) -> io::Result<bool> {
    let mut flags = 0;
    // SAFETY: the handle is live, the flags pointer is writable, and the other
    // outputs are optional according to GetVolumeInformationByHandleW.
    success(unsafe {
        GetVolumeInformationByHandleW(
            file.as_raw_handle(),
            ptr::null_mut(),
            0,
            ptr::null_mut(),
            ptr::null_mut(),
            &mut flags,
            ptr::null_mut(),
            0,
        )
    })?;
    Ok(flags & FILE_PERSISTENT_ACLS != 0)
}

struct Descriptor {
    memory: *mut c_void,
    dacl: Acl,
    control: u16,
}

impl Drop for Descriptor {
    fn drop(&mut self) {
        if !self.memory.is_null() {
            // SAFETY: GetSecurityInfo allocates this descriptor with LocalAlloc.
            unsafe { LocalFree(self.memory) };
        }
    }
}

#[derive(Debug, PartialEq, Eq)]
struct Signature {
    present: bool,
    protected: bool,
    // None is a NULL DACL, distinct from an empty DACL (Some(empty)).
    aces: Option<Vec<Vec<u8>>>,
}

impl Descriptor {
    fn read(file: &File) -> io::Result<Self> {
        let mut descriptor = Self {
            memory: ptr::null_mut(),
            dacl: ptr::null_mut(),
            control: 0,
        };
        // SAFETY: all supplied outputs are writable. The DACL pointer is owned
        // by the returned security descriptor, which stays alive until Drop.
        status(unsafe {
            GetSecurityInfo(
                file.as_raw_handle(),
                SE_FILE_OBJECT,
                DACL_SECURITY_INFORMATION,
                ptr::null_mut(),
                ptr::null_mut(),
                &mut descriptor.dacl,
                ptr::null_mut(),
                &mut descriptor.memory,
            )
        })?;
        if descriptor.memory.is_null() {
            return Err(io::Error::other("Missing Windows security descriptor"));
        }
        let mut revision = 0;
        success(unsafe {
            GetSecurityDescriptorControl(descriptor.memory, &mut descriptor.control, &mut revision)
        })?;
        Ok(descriptor)
    }

    fn signature(&self) -> io::Result<Signature> {
        let aces = if self.dacl.is_null() {
            None
        } else {
            let mut info = AclSizeInformation::default();
            success(unsafe {
                GetAclInformation(
                    self.dacl,
                    (&mut info as *mut AclSizeInformation).cast(),
                    std::mem::size_of::<AclSizeInformation>() as u32,
                    ACL_SIZE_INFORMATION_CLASS,
                )
            })?;
            let mut aces = Vec::new();
            for index in 0..info.ace_count {
                let mut ace = ptr::null_mut();
                success(unsafe { GetAce(self.dacl, index, &mut ace) })?;
                if ace.is_null() {
                    return Err(io::Error::other("Missing Windows ACE"));
                }
                // ACE_HEADER consists of type, flags, and a little-endian u16 size.
                // GetAce returns a pointer within the valid ACL owned by self.
                let header = unsafe { std::slice::from_raw_parts(ace.cast::<u8>(), 4) };
                let size = u16::from_le_bytes([header[2], header[3]]) as usize;
                if size < 4 || size > info.bytes_in_use as usize {
                    return Err(io::Error::other("Invalid Windows ACE size"));
                }
                let bytes = unsafe { std::slice::from_raw_parts(ace.cast::<u8>(), size) };
                aces.push(bytes.to_vec());
            }
            Some(aces)
        };
        Ok(Signature {
            present: self.control & SE_DACL_PRESENT != 0,
            protected: self.control & SE_DACL_PROTECTED != 0,
            aces,
        })
    }

    fn apply(&self, file: &File) -> io::Result<()> {
        let inheritance = if self.control & SE_DACL_PROTECTED != 0 {
            PROTECTED_DACL_SECURITY_INFORMATION
        } else {
            UNPROTECTED_DACL_SECURITY_INFORMATION
        };
        // SAFETY: the handle grants WRITE_DAC and the descriptor owns its DACL.
        status(unsafe {
            SetSecurityInfo(
                file.as_raw_handle(),
                SE_FILE_OBJECT,
                DACL_SECURITY_INFORMATION | inheritance,
                ptr::null_mut(),
                ptr::null_mut(),
                self.dacl,
                ptr::null_mut(),
            )
        })
    }
}

/// Holds the original and replacement open with shared-delete access until the
/// final check. This is not a filesystem compare-and-swap operation.
pub struct PreservedAcl {
    source: File,
    destination: File,
    original: Signature,
}

impl PreservedAcl {
    pub fn prepare(source: &Path, destination: &Path) -> io::Result<Option<Self>> {
        let source = open_security(source, READ_CONTROL)?;
        if !supports_acls(&source)? {
            // FAT/exFAT and other volumes without persistent ACLs have no DACL to copy.
            return Ok(None);
        }
        let destination = open_security(destination, READ_CONTROL | WRITE_DAC)?;
        let descriptor = Descriptor::read(&source)?;
        let original = descriptor.signature()?;
        if Descriptor::read(&destination)?.signature()? != original {
            descriptor.apply(&destination)?;
        }
        let preserved = Self {
            source,
            destination,
            original,
        };
        preserved.check()?;
        Ok(Some(preserved))
    }

    fn check(&self) -> io::Result<()> {
        if Descriptor::read(&self.source)?.signature()? != self.original {
            return Err(io::Error::other("Windows ACL changed during save"));
        }
        if Descriptor::read(&self.destination)?.signature()? != self.original {
            return Err(io::Error::other("Could not preserve Windows file ACL"));
        }
        Ok(())
    }

    pub fn verify(self) -> io::Result<()> {
        self.check()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn read_acl(path: &Path) -> Signature {
        Descriptor::read(&open_security(path, READ_CONTROL).unwrap())
            .unwrap()
            .signature()
            .unwrap()
    }

    #[test]
    fn preserves_inherited_acl() {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("note.txt");
        std::fs::write(&path, "original").unwrap();
        let file = open_security(&path, READ_CONTROL).unwrap();
        assert!(supports_acls(&file).unwrap(), "Windows CI requires NTFS");
        drop(file);
        let before = read_acl(&path);
        let mut document = crate::document::Document::open(&path).unwrap();
        document.save(&path, "changed").unwrap();
        assert_eq!(std::fs::read_to_string(&path).unwrap(), "changed");
        assert_eq!(read_acl(&path), before);
    }

    #[test]
    fn preserves_explicit_deny_and_protected_acl() {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("not Türkçe.txt");
        std::fs::write(&path, "original").unwrap();
        // Anonymous Logon is a well-known SID, independent of Windows language.
        // Deny this unrelated account, not the test process itself.
        let output = std::process::Command::new("icacls.exe")
            .arg(&path)
            .args(["/inheritance:d", "/deny", "*S-1-5-7:(R)"])
            .output()
            .unwrap();
        assert!(output.status.success(), "icacls failed: {output:?}");
        let before = read_acl(&path);
        assert!(before.protected);
        assert!(before
            .aces
            .as_ref()
            .is_some_and(|aces| aces.iter().any(|ace| ace[0] == 1)));
        let mut document = crate::document::Document::open(&path).unwrap();
        document.save(&path, "changed").unwrap();
        assert_eq!(std::fs::read_to_string(&path).unwrap(), "changed");
        assert_eq!(read_acl(&path), before);
    }
}
