use crate::process::fd_table::Fd;
use core::ffi::c_char;
use libkernel::error::{KernelError, Result};
use libkernel::memory::address::TUA;

pub async fn sys_readlinkat(
    _dirfd: Fd,
    _path: TUA<c_char>,
    _statbuf: TUA<c_char>,
    _size: usize,
) -> Result<usize> {
    // TODO: This is safe for fat32, since it doesn't support symbolic links.
    // However, we need to implement this for a real FS!
    Err(KernelError::InvalidValue)
}
