use crate::process::Task;
use alloc::sync::Arc;
use libkernel::UserAddressSpace;

pub mod idle;
pub mod signal;

/// 架构相关的上下文切换
///
/// 主要负责切换进程地址空间（页表）。
///
/// 在 RISC-V 中：
/// 1. 获取新进程的 VM 锁并关中断 (`lock_save_irq`)。
/// 2. 获取地址空间的可变引用。
/// 3. 调用 `activate()`，这将写入 `satp` 寄存器并执行 `sfence.vma` 刷新 TLB。
pub fn context_switch(new: Arc<Task>) {
    new.vm
        .lock_save_irq()
        .mm_mut()
        .address_space_mut()
        .activate();
}