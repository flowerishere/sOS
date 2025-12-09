// libkernel/src/arch/riscv64/mod.rs

use riscv::register::{sstatus, sie};
// 注意：SpinLockIrq 的引用路径可能需要根据你的 libkernel 结构调整
// 如果 sync 模块在 libkernel 根目录：
use crate::{CpuOps, VirtualMemory, KernAddressSpace, sync::spinlock::SpinLockIrq};

// 1. 删除 pub mod boot; 和 pub mod fdt;
// 2. 保留实现 Arch Trait 必须的模块
pub mod cpu_ops;   // 实现 CpuOps
pub mod exceptions; // 实现 UserContext (TrapFrame)
pub mod memory;    // 实现 VirtualMemory

// 定义架构结构体
pub struct Riscv64;

// 实现 VirtualMemory Trait (这是 libkernel 必须的)
impl VirtualMemory for Riscv64 {
    type PageTableRoot = memory::pg_tables::L2Table; // Sv39 Root
    type ProcessAddressSpace = memory::mmu::RiscvProcessAddressSpace;
    type KernelAddressSpace = memory::mmu::RiscvKernelAddressSpace;

    // Sv39 线性映射偏移量 (Upper half kernel)
    const PAGE_OFFSET: usize = 0xffff_ffc0_0000_0000; 

    fn kern_address_space() -> &'static SpinLockIrq<Self::KernelAddressSpace, Self> {
        &memory::mmu::KERN_ADDR_SPACE
    }
}

impl crate::arch::Arch for Riscv64 {
    type UserContext = exceptions::TrapFrame;

    fn name() -> &'static str {
        "riscv64"
    }

    fn new_user_context(entry_point: crate::memory::address::VA, stack_top: crate::memory::address::VA) -> Self::UserContext {
        exceptions::TrapFrame::new_user(entry_point, stack_top)
    }

    fn power_off() -> ! {
        sbi_rt::system_reset(sbi_rt::Shutdown, sbi_rt::NoReason);
        loop { unsafe { riscv::asm::wfi() }; }
    }

    fn do_signal_return() -> impl core::future::Future<Output = crate::error::Result<Self::UserContext>> {
        async { Err(crate::error::KernelError::NotImplemented) }
    }

    unsafe fn copy_from_user(_src: crate::memory::address::UA, _dst: *mut (), _len: usize) -> impl core::future::Future<Output = crate::error::Result<()>> {
        async { Ok(()) }
    }

    unsafe fn copy_to_user(_src: *const (), _dst: crate::memory::address::UA, _len: usize) -> impl core::future::Future<Output = crate::error::Result<()>> {
        async { Ok(()) }
    }

    unsafe fn copy_strn_from_user(_src: crate::memory::address::UA, _dst: *mut u8, _len: usize) -> impl core::future::Future<Output = crate::error::Result<usize>> {
        async { Ok(0) }
    }
}