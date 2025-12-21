use alloc::sync::Arc;
use core::future::Future;
use riscv::{
    asm::wfi,
    register::sstatus,
};

use cpu_ops::{local_irq_restore, local_irq_save};
use exceptions::TrapFrame;
use libkernel::{
    CpuOps, VirtualMemory,
    arch::riscv64::memory::pg_tables::RvPageTableRoot,
    error::Result,
    memory::address::{UA, VA},
};
use memory::{
    PAGE_OFFSET,
    address_space::RiscvProcessAddressSpace,
    mmu::{RiscvKernelAddressSpace, KERN_ADDR_SPACE},
    uaccess::{RiscvCopyFromUser, RiscvCopyStrnFromUser, RiscvCopyToUser},
};

use crate::{
    process::{
        Task,
        thread_group::signal::{SigId, ksigaction::UserspaceSigAction},
    },
    sync::SpinLock,
};

use super::Arch;

pub mod boot;
mod cpu_ops;
pub mod exceptions;
// mod fdt; // 如果你有 RISC-V 特定的 FDT 处理逻辑，取消注释
mod memory;
mod proc;
pub mod sbi; // 建议创建一个简单的 sbi.rs 模块或直接使用 sbi-rt

/// RISC-V 64 Architecture Provider
pub struct Riscv64;



impl VirtualMemory for Riscv64 {
    // RISC-V SV39/48 的页表根类型
    type PageTableRoot = RvPageTableRoot;
    type ProcessAddressSpace = RiscvProcessAddressSpace;
    type KernelAddressSpace = RiscvKernelAddressSpace;

    const PAGE_OFFSET: usize = PAGE_OFFSET;

    fn kern_address_space() -> &'static SpinLock<Self::KernelAddressSpace> {
        KERN_ADDR_SPACE.get().unwrap()
    }
}

impl Arch for Riscv64 {
    // 使用 TrapFrame 作为用户上下文 (保存通用寄存器 + CSRs)
    type UserContext = TrapFrame;

    fn new_user_context(entry_point: VA, stack_top: VA) -> Self::UserContext {
        let mut ctx = TrapFrame::default();
        
        // 设置程序计数器 (sepc)
        ctx.sepc = entry_point.value();
        
        // 设置栈指针 (x2/sp)
        ctx.x[2] = stack_top.value();
        
        // 设置初始状态：
        // SPIE (Bit 5) = 1: 确保 sret 返回用户态后开启中断
        // SPP (Bit 8) = 0: 之前的特权级是 User Mode
        ctx.sstatus = 1 << 5; 
        
        ctx
    }

    fn name() -> &'static str {
        "riscv64"
    }

    fn do_signal(
        sig: SigId,
        action: UserspaceSigAction,
    ) -> impl Future<Output = Result<<Self as Arch>::UserContext>> {
        proc::signal::do_signal(sig, action)
    }

    fn do_signal_return() -> impl Future<Output = Result<<Self as Arch>::UserContext>> {
        proc::signal::do_signal_return()
    }

    fn context_switch(new: Arc<Task>) {
        proc::context_switch(new);
    }

    fn create_idle_task() -> Task {
        proc::idle::create_idle_task()
    }

    fn power_off() -> ! {
        // 使用 SBI System Reset Extension 进行关机
        // 0x53525354 = 'SRST' System Reset Extension
        // Type: Shutdown (0), Reason: NoReason (0)
        // 注意：需要确保 Cargo.toml 中引入了 `sbi-rt`
        #[cfg(feature = "sbi-rt")]
        sbi_rt::system_reset(sbi_rt::Shutdown, sbi_rt::NoReason);

        // 如果没有 SBI 或返回了，进入死循环
        Self::halt()
    }

    unsafe fn copy_from_user(
        src: UA,
        dst: *mut (),
        len: usize,
    ) -> impl Future<Output = Result<()>> {
        RiscvCopyFromUser::new(src, dst, len)
    }

    unsafe fn copy_to_user(
        src: *const (),
        dst: UA,
        len: usize,
    ) -> impl Future<Output = Result<()>> {
        RiscvCopyToUser::new(src, dst, len)
    }

    unsafe fn copy_strn_from_user(
        src: UA,
        dst: *mut u8,
        len: usize,
    ) -> impl Future<Output = Result<usize>> {
        RiscvCopyStrnFromUser::new(src, dst as *mut _, len)
    }
}