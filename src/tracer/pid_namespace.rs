use std::fs;
use std::os::fd::AsRawFd;
use std::os::unix::fs::MetadataExt;

use anyhow::{Context, Result};

const PID_NS_PATH: &str = "/proc/self/ns/pid";

// Linux UAPI: include/uapi/linux/nsfs.h, enum init_ns_ino.
// Stable inode assigned to the initial PID namespace by nsfs.
const PID_NS_INIT_INO: u32 = 0xEFFFFFFC;

// Linux UAPI: include/uapi/linux/nsfs.h, enum init_ns_id.
// Stable namespace ID returned by NS_GET_ID for the initial PID namespace.
const PID_NS_INIT_ID: u64 = 4;

// Linux UAPI: include/uapi/linux/nsfs.h
const NSFS_IOCTL_TYPE: u8 = 0xb7;
const NS_GET_ID: u8 = 13;
nix::ioctl_read!(read_namespace_id, NSFS_IOCTL_TYPE, NS_GET_ID, u64);

#[derive(Clone, Copy)]
enum CurrentPidNamespace {
    Initial,
    Nested { inum: u32 },
    Unknown { inum: u32 },
    TargetOverride { inum: u32 },
}

pub struct Detection {
    current_pid_namespace: CurrentPidNamespace,
}

impl Detection {
    pub fn with_target_override(pidns_inum: u32) -> Self {
        Self {
            current_pid_namespace: CurrentPidNamespace::TargetOverride { inum: pidns_inum },
        }
    }

    pub fn current_is_init_pidns(&self) -> bool {
        matches!(self.current_pid_namespace, CurrentPidNamespace::Initial)
    }

    pub fn current_pidns_inum(&self) -> Option<u32> {
        self.current_pid_namespace.inum()
    }

    pub fn log(&self) {
        match self.current_pid_namespace {
            CurrentPidNamespace::Initial => {
                eprintln!("LiME: current PID namespace: initial");
                eprintln!("LiME: PID translation: using direct task pid/tgid fast path");
            }
            CurrentPidNamespace::Nested { inum } => {
                eprintln!("LiME: current PID namespace: nested (inode {inum})");
                eprintln!(
                    "LiME: PID translation: resolving kernel task PIDs into the current namespace"
                );
            }
            CurrentPidNamespace::Unknown { inum } => {
                eprintln!("LiME: current PID namespace: unknown (inode {inum})");
                eprintln!(
                    "LiME: PID translation: treating current namespace as nested and resolving kernel task PIDs"
                );
            }
            CurrentPidNamespace::TargetOverride { inum } => {
                eprintln!("LiME: target PID namespace override: inode {inum}");
                eprintln!(
                    "LiME: PID translation: resolving kernel task PIDs into the target namespace"
                );
            }
        }
    }
}

pub fn detect() -> Result<Detection> {
    Ok(Detection {
        current_pid_namespace: CurrentPidNamespace::detect()?,
    })
}

impl CurrentPidNamespace {
    fn detect() -> Result<Self> {
        let inum = current_pidns_inum()?;

        Ok(match current_pidns_id() {
            Some(PID_NS_INIT_ID) => Self::Initial,
            Some(_) => Self::Nested { inum },
            None if inum == PID_NS_INIT_INO => Self::Initial,
            None => Self::Unknown { inum },
        })
    }

    fn inum(&self) -> Option<u32> {
        match *self {
            Self::Initial => None,
            Self::Nested { inum } | Self::Unknown { inum } | Self::TargetOverride { inum } => {
                Some(inum)
            }
        }
    }
}

fn current_pidns_id() -> Option<u64> {
    let pidns = fs::File::open(PID_NS_PATH).ok()?;
    let mut id = 0_u64;

    // `pidns` is opened from nsfs, and `id` is a valid out-pointer for
    // the u64 value written by the NS_GET_ID ioctl.
    unsafe { read_namespace_id(pidns.as_raw_fd(), &mut id).ok()? };

    Some(id)
}

fn current_pidns_inum() -> Result<u32> {
    let inum = fs::metadata(PID_NS_PATH)
        .with_context(|| format!("failed to read current PID namespace from {PID_NS_PATH}"))?;
    let inum = inum.ino();

    u32::try_from(inum)
        .with_context(|| format!("current PID namespace inode {inum} does not fit in u32"))
}
