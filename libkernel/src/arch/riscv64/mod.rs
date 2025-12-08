use core::future::Future;
use riscv::register::{sstatus, sie};

// We temporarily comment out process imports since they are missing in libkernel root
// use crate::process::{Task, thread_group::signal::{SigId, ksigaction::UserspaceSigAction}};
// use alloc::sync::Arc;

use crate::{
    CpuOps, VirtualMemory,
    error::{Result, KernelError},
    memory::address::{UA, VA},
    sync::spinlock::Spinlock,
    KernAddressSpace,
};

// Declare submodules
pub mod boot;
pub mod memory;
pub mod exceptions;
pub mod cpu_ops;
pub mod fdt;

// Re-export boot functions for the assembly entry point
pub use boot::*; 

/// The RISC-V 64 architecture implementation struct.
pub struct Riscv64;

impl CpuOps for Riscv64 {
    #[inline(always)]
    fn id() -> usize {
        // In S-Mode on QEMU/Virt, Hart ID is often passed in a0 or stored in tp
        // We assume it was saved to tp during boot
        let hartid: usize;
        unsafe { core::arch::asm!("mv {}, tp", out(reg) hartid) };
        hartid
    }

    #[inline(always)]
    fn enable_interrupts() {
        unsafe { sstatus::set_sie() };
    }

    #[inline(always)]
    fn disable_interrupts() {
        unsafe { sstatus::clear_sie() };
    }

    #[inline(always)]
    fn irq_enabled() -> bool {
        sstatus::read().sie()
    }
}

impl VirtualMemory for Riscv64 {
    fn kern_address_space() -> &'static Spinlock<KernAddressSpace> {
        &memory::mmu::KERN_ADDR_SPACE
    }
}

impl crate::arch::Arch for Riscv64 {
    type UserContext = exceptions::TrapFrame;

    fn name() -> &'static str {
        "riscv64"
    }

    fn new_user_context(entry_point: VA, stack_top: VA) -> Self::UserContext {
        exceptions::TrapFrame::new_user(entry_point, stack_top)
    }

    // Methods commented out in trait definition are omitted here:
    // context_switch, create_idle_task, do_signal

    fn power_off() -> ! {
        sbi_rt::system_reset(sbi_rt::Shutdown, sbi_rt::NoReason);
        loop { unsafe { riscv::asm::wfi() }; }
    }

    fn do_signal_return() -> impl Future<Output = Result<<Self as crate::arch::Arch>::UserContext>> {
        // Placeholder implementation
        async { Err(KernelError::NotImplemented) }
    }

    unsafe fn copy_from_user(_src: UA, _dst: *mut (), _len: usize) -> impl Future<Output = Result<()>> {
        async { Ok(()) } 
    }

    unsafe fn copy_to_user(_src: *const (), _dst: UA, _len: usize) -> impl Future<Output = Result<()>> {
        async { Ok(()) }
    }

    unsafe fn copy_strn_from_user(
        _src: UA,
        _dst: *mut u8,
        _len: usize,
    ) -> impl Future<Output = Result<usize>> {
        async { Ok(0) }
    }
}