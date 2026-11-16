//! Files a sync app keeps in the cloud, which macOS hands over only to a process that asks.
//!
//! A folder synced on demand (iCloud Drive, Synology Drive, OneDrive) shows files whose bytes are not on the
//! disk. The system downloads one as it is read, but only for a process whose policy says so: any other is
//! refused with `EDEADLK`. An app started from Finder is given that policy, and one started any other way
//! has its parent's.

use std::ffi::c_int;

// From `<sys/resource.h>`.
const IOPOL_TYPE_VFS_MATERIALIZE_DATALESS_FILES: c_int = 3;
const IOPOL_SCOPE_PROCESS: c_int = 0;
const IOPOL_MATERIALIZE_DATALESS_FILES_ON: c_int = 2;

unsafe extern "C" {
    fn setiopolicy_np(iotype: c_int, scope: c_int, policy: c_int) -> c_int;
}

/// Asks the system to download a file kept in the cloud when this process reads it, however the process
/// was started. `false` says it would not, and such a file then fails to read as it did before.
pub fn fetch_cloud_files() -> bool {
    // SAFETY: the call takes three integers and no memory, and sets a flag on this process alone. The
    // signature is the one `<sys/resource.h>` declares, in libSystem since macOS 10.5.
    let refused = unsafe {
        setiopolicy_np(
            IOPOL_TYPE_VFS_MATERIALIZE_DATALESS_FILES,
            IOPOL_SCOPE_PROCESS,
            IOPOL_MATERIALIZE_DATALESS_FILES_ON,
        )
    };
    refused == 0
}
