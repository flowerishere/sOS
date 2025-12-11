//! The architectural abstraction layer.

use crate::process::{
    Task,
    thread_group::signal::{SigId, ksigaction::UserspaceSigAction},
};
use alloc::sync::Arc;
use core::future::Future;
use libkernel::{
    CpuOps, VirtualMemory,
    error::Result,
    memory::address::{UA, VA},
};

pub trait Arch: CpuOps + VirtualMemory {
    type UserContext: Sized + Send + Sync + Clone;

    fn name() -> &'static str;
    fn new_user_context(entry_point: VA, stack_top: VA) -> Self::UserContext;
    fn context_switch(new: Arc<Task>);
    fn create_idle_task() -> Task;
    fn power_off() -> !;
    fn do_signal(
        sig: SigId,
        action: UserspaceSigAction,
    ) -> impl Future<Output = Result<<Self as Arch>::UserContext>>;
    fn do_signal_return() -> impl Future<Output = Result<<Self as Arch>::UserContext>>;

    unsafe fn copy_from_user(src: UA, dst: *mut (), len: usize) -> impl Future<Output = Result<()>>;
    unsafe fn copy_to_user(src: *const (), dst: UA, len: usize) -> impl Future<Output = Result<()>>;
    unsafe fn copy_strn_from_user(src: UA, dst: *mut u8, len: usize) -> impl Future<Output = Result<usize>>;
}

// --- Architecture Specific Modules ---

#[cfg(any(feature = "arch-aarch64", doc))]
pub mod arm64;

#[cfg(feature = "arch-aarch64")]
pub use self::arm64::Aarch64 as ArchImpl;


#[cfg(any(feature = "arch-riscv64", doc))]
pub mod riscv64;

#[cfg(feature = "arch-riscv64")]
pub use self::riscv64::Riscv64 as ArchImpl;