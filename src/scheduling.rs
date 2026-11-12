/// Leave CPU time available for the compositor and interactive desktop processes.
#[rustfmt::skip]
pub fn lower_priority()
{
    #[cfg(unix)]
    unsafe
    {
        // Desktop launchers may themselves have negative niceness. Use an absolute floor
        // so inheriting that elevated priority cannot keep the editor above normal.
        if libc::getpriority(libc::PRIO_PROCESS, 0) < 5
        {
            libc::setpriority(libc::PRIO_PROCESS, 0, 5);
        }
    }

    #[cfg(target_os = "windows")]
    unsafe
    {
        #[link(name = "kernel32")]
        extern "system"
        {
            fn GetCurrentProcess() -> *mut core::ffi::c_void;
            fn SetPriorityClass(process: *mut core::ffi::c_void, class: u32) -> i32;
        }

        const BELOW_NORMAL_PRIORITY_CLASS: u32 = 0x00004000;
        SetPriorityClass(GetCurrentProcess(), BELOW_NORMAL_PRIORITY_CLASS);
    }
}
