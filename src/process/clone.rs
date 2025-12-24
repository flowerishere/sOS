use crate::{
    process::{TASK_LIST, Task, TaskState},
    sched::{self, current_task},
    sync::SpinLock,
};
use bitflags::bitflags;
use libkernel::{
    error::{KernelError, Result},
    memory::address::UA,
};
use ringbuf::Arc;

use super::{ctx::Context, thread_group::signal::SigSet};

bitflags! {
    #[derive(Debug, Clone, Copy)]
    pub struct CloneFlags: u32 {
        const CLONE_VM = 0x100;
        const CLONE_FS = 0x200;
        const CLONE_FILES = 0x400;
        const CLONE_SIGHAND = 0x800;
        const CLONE_PTRACE = 0x2000;
        const CLONE_VFORK = 0x4000;
        const CLONE_PARENT = 0x8000;
        const CLONE_THREAD = 0x10000;
        const CLONE_NEWNS = 0x20000;
        const CLONE_SYSVSEM = 0x40000;
        const CLONE_SETTLS = 0x80000;
        const CLONE_PARENT_SETTID = 0x100000;
        const CLONE_CHILD_CLEARTID = 0x200000;
        const CLONE_DETACHED = 0x400000;
        const CLONE_UNTRACED = 0x800000;
        const CLONE_CHILD_SETTID = 0x01000000;
        const CLONE_NEWCGROUP = 0x02000000;
        const CLONE_NEWUTS = 0x04000000;
        const CLONE_NEWIPC = 0x08000000;
        const CLONE_NEWUSER = 0x10000000;
        const CLONE_NEWPID = 0x20000000;
        const CLONE_NEWNET = 0x40000000;
        const CLONE_IO = 0x80000000;
    }
}

pub async fn sys_clone(
    flags: u32,
    newsp: usize,
    _parent_tidptr: UA,
    _child_tidptr: UA,
    tls: usize,
) -> Result<usize> {
    let flags = CloneFlags::from_bits_truncate(flags);

    let new_task = {
        let current_task = current_task();

        // 处理线程组和父子进程关系
        let (tg, tid) = if flags.contains(CloneFlags::CLONE_THREAD) {
            // CLONE_THREAD 要求必须同时设置 CLONE_SIGHAND 和 CLONE_VM
            if !flags.contains(CloneFlags::CLONE_SIGHAND | CloneFlags::CLONE_VM) {
                return Err(KernelError::InvalidValue);
            }
            (
                // 在当前线程组内创建新任务
                current_task.process.clone(),
                current_task.process.next_tid(),
            )
        } else {
            let tgid_parent = if flags.contains(CloneFlags::CLONE_PARENT) {
                // 使用父进程的父进程作为新父进程
                current_task
                    .process
                    .parent
                    .lock_save_irq()
                    .clone()
                    .and_then(|p| p.upgrade())
                    // 不能对 init 进程使用 CLONE_PARENT
                    .ok_or(KernelError::InvalidValue)?
            } else {
                current_task.process.clone()
            };

            tgid_parent.new_child(flags.contains(CloneFlags::CLONE_SIGHAND))
        };

        // 处理虚拟内存 (VM)
        let vm = if flags.contains(CloneFlags::CLONE_VM) {
            current_task.vm.clone()
        } else {
            Arc::new(SpinLock::new(
                current_task.vm.lock_save_irq().clone_as_cow()?,
            ))
        };

        // 处理文件描述符表
        let files = if flags.contains(CloneFlags::CLONE_FILES) {
            current_task.fd_table.clone()
        } else {
            Arc::new(SpinLock::new(
                current_task.fd_table.lock_save_irq().clone_for_exec(),
            ))
        };

        // 处理当前工作目录
        let cwd = if flags.contains(CloneFlags::CLONE_FS) {
            current_task.cwd.clone()
        } else {
            Arc::new(SpinLock::new(current_task.cwd.lock_save_irq().clone()))
        };

        let creds = current_task.creds.lock_save_irq().clone();

        // ====================================================================
        // 关键修改：架构相关的上下文设置 (Context Setup)
        // ====================================================================
        
        // 获取父进程的上下文快照
        let mut user_ctx = *current_task.ctx.lock_save_irq().user();

        // ----------------------- 针对 AArch64 (ARM64) -----------------------
        #[cfg(target_arch = "aarch64")]
        {
            // 1. 设置子进程返回值为 0 (x0 寄存器)
            user_ctx.x[0] = 0;

            // 2. 设置栈指针 (SP)
            if newsp != 0 {
                // ARM64 TrapFrame 通常有独立的 sp 字段，或者是 regs[31]
                // 这里假设你的 TrapFrame 定义中有 sp 字段
                user_ctx.sp = newsp as u64; 
            }

            // 3. 设置 TLS (Thread Local Storage)
            if flags.contains(CloneFlags::CLONE_SETTLS) {
                // ARM64 使用 tpidr_el0 系统寄存器
                user_ctx.tpidr = tls as u64;
            }
        }

        // -------------------- 针对 RISC-V (64位或32位) --------------------
        #[cfg(any(target_arch = "riscv64", target_arch = "riscv32"))]
        {
            // 1. 设置子进程返回值为 0 (a0 = x10 寄存器)
            user_ctx.regs[10] = 0;

            // 2. 设置栈指针 (SP = x2 寄存器)
            if newsp != 0 {
                user_ctx.regs[2] = newsp;
            }

            // 3. 设置 TLS (Thread Pointer = tp = x4 寄存器)
            if flags.contains(CloneFlags::CLONE_SETTLS) {
                user_ctx.regs[4] = tls;
            }
        }

        // ====================================================================

        let new_sigmask = *current_task.sig_mask.lock_save_irq();

        Task {
            tid,
            process: tg,
            vm,
            fd_table: files,
            cwd,
            creds: SpinLock::new(creds),
            ctx: SpinLock::new(Context::from_user_ctx(user_ctx)),
            priority: current_task.priority,
            sig_mask: SpinLock::new(new_sigmask),
            pending_signals: SpinLock::new(SigSet::empty()),
            vruntime: SpinLock::new(*current_task.vruntime.lock_save_irq()),
            exec_start: SpinLock::new(None),
            deadline: SpinLock::new(*current_task.deadline.lock_save_irq()),
            state: Arc::new(SpinLock::new(TaskState::Runnable)),
            last_run: SpinLock::new(None),
            robust_list: SpinLock::new(None),
        }
    };

    TASK_LIST
        .lock_save_irq()
        .insert(new_task.descriptor(), Arc::downgrade(&new_task.state));

    let tid = new_task.tid;

    sched::insert_task(Arc::new(new_task));

    Ok(tid.value() as _)
}