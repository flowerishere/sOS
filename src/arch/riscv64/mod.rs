// src/arch/riscv64/mod.rs

use alloc::sync::Arc;
use core::future::Future;
use libkernel::{
    CpuOps, VirtualMemory,
    arch::riscv64::memory::{
        pg_tables::L2Table, // Sv39 使用 L2Table 作为根页表
        mmu::{RiscvProcessAddressSpace, RiscvKernelAddressSpace, KERN_ADDR_SPACE},
    },
    error::Result,
    memory::address::{UA, VA},
};
use riscv::{asm::wfi, register::sstatus};

use crate::{
    process::{
        Task,
        thread_group::signal::{SigId, ksigaction::UserspaceSigAction},
    },
    sync::SpinLock,
};

// 引入上层 Arch Trait 定义
use super::Arch;

// --- 模块定义 ---
// 这些模块需要在 src/arch/riscv64/ 目录下创建对应的文件或文件夹
mod boot;
mod cpu_ops;
mod exceptions;
mod fdt;
mod memory;
mod proc;
pub mod smp;
// cpu_ops 模块如果逻辑简单，可以直接在本文件中实现，或者单独分文件
// 这里我们参考 arm64 的结构保留模块定义，但主要逻辑在下方 impl 中实现
pub mod cpu_ops; 

/// RISC-V 64 架构实现结构体
pub struct Riscv64;



// 2. 实现虚拟内存接口 (VirtualMemory)
impl VirtualMemory for Riscv64 {
    // RISC-V Sv39 三级页表：L2 -> L1 -> L0
    type PageTableRoot = L2Table;
    type ProcessAddressSpace = RiscvProcessAddressSpace;
    type KernelAddressSpace = RiscvKernelAddressSpace;

    // RISC-V Sv39 内核线性映射偏移量 (0xffff_ffc0_0000_0000)
    // 需确保与 libkernel 中的定义一致
    const PAGE_OFFSET: usize = 0xffff_ffc0_0000_0000;

    fn kern_address_space() -> &'static SpinLock<Self::KernelAddressSpace> {
        &KERN_ADDR_SPACE
    }
}

// 3. 实现核心 Arch Trait
impl Arch for Riscv64 {
    // 用户上下文使用 TrapFrame (通常包含通用寄存器、sepc, sstatus 等)
    type UserContext = libkernel::arch::riscv64::exceptions::TrapFrame;

    fn name() -> &'static str {
        "riscv64"
    }

    fn new_user_context(entry_point: VA, stack_top: VA) -> Self::UserContext {
        // 调用 libkernel 中 TrapFrame 的构造函数
        Self::UserContext::new_user(entry_point, stack_top)
    }

    fn context_switch(new: Arc<Task>) {
        // 调用 proc 模块实现的上下文切换逻辑
        proc::context_switch(new);
    }

    fn create_idle_task() -> Task {
        // 调用 proc 模块创建 idle 任务
        proc::idle::create_idle_task()
    }

    fn power_off() -> ! {
        // 使用 SBI 调用进行关机 (System Reset Extension)
        sbi_rt::system_reset(sbi_rt::Shutdown, sbi_rt::NoReason);
        
        // 如果 SBI 调用返回或失败，死循环halt
        Self::halt()
    }

    // 信号处理相关，委托给 proc::signal 模块
    fn do_signal(
        sig: SigId,
        action: UserspaceSigAction,
    ) -> impl Future<Output = Result<<Self as Arch>::UserContext>> {
        proc::signal::do_signal(sig, action)
    }

    fn do_signal_return() -> impl Future<Output = Result<<Self as Arch>::UserContext>> {
        proc::signal::do_signal_return()
    }

    // 用户态内存访问 (User Access)
    // 委托给 memory::uaccess 模块实现的 Future 类型
    unsafe fn copy_from_user(
        src: UA,
        dst: *mut (),
        len: usize,
    ) -> impl Future<Output = Result<()>> {
        memory::uaccess::Riscv64CopyFromUser::new(src, dst, len)
    }

    unsafe fn copy_to_user(
        src: *const (),
        dst: UA,
        len: usize,
    ) -> impl Future<Output = Result<()>> {
        memory::uaccess::Riscv64CopyToUser::new(src, dst, len)
    }

    unsafe fn copy_strn_from_user(
        src: UA,
        dst: *mut u8,
        len: usize,
    ) -> impl Future<Output = Result<usize>> {
        memory::uaccess::Riscv64CopyStrnFromUser::new(src, dst, len)
    }
}