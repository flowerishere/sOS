use crate::{
    arch::riscv64::TrapFrame,
    memory::uaccess::{UserCopyable, copy_from_user, copy_to_user},
    process::thread_group::signal::{
        SigId, ksigaction::UserspaceSigAction, sigaction::SigActionFlags,
    },
    sched::current_task,
};
use libkernel::{
    error::Result,
    memory::{
        PAGE_SIZE,
        address::{TUA, UA},
    },
};

#[repr(C)]
#[derive(Clone, Copy)]
struct RtSigFrame {
    uctx: TrapFrame,
    alt_stack_prev_addr: UA,
}

// SAFETY: RtSigFrame 只包含 POD 数据，可以安全地在用户态和内核态之间拷贝
unsafe impl UserCopyable for RtSigFrame {}

pub async fn do_signal(id: SigId, sa: UserspaceSigAction) -> Result<TrapFrame> {
    let task = current_task();
    let mut signal = task.process.signals.lock_save_irq();

    // 获取当前任务保存的用户态上下文
    let saved_state = *task.ctx.lock_save_irq().user();
    let mut new_state = saved_state.clone();
    
    let mut frame = RtSigFrame {
        uctx: saved_state,
        alt_stack_prev_addr: UA::null(),
    };

    if !sa.flags.contains(SigActionFlags::SA_RESTORER) {
        panic!("Cannot call non-sa_restorer sig handler");
    }

    // 确定信号栈地址
    let addr: TUA<RtSigFrame> = if sa.flags.contains(SigActionFlags::SA_ONSTACK)
        && let Some(alt_stack) = signal.alt_stack.as_mut()
        && let Some(alloc) = alt_stack.alloc_alt_stack::<RtSigFrame>()
    {
        frame.alt_stack_prev_addr = alloc.old_ptr;
        alloc.data_ptr.cast()
    } else {
        // 使用当前栈顶向下分配
        // regs[2] 是 RISC-V 的 sp 寄存器
        TUA::from_value(new_state.regs[2] as _)
            .sub_objs(1)
            .align(PAGE_SIZE)
    };

    // 将 Signal Frame 压入用户栈
    copy_to_user(addr, frame).await?;

    // 设置新的上下文以跳转到信号处理函数
    // 1. 设置 sp (x2) 指向新的栈顶
    new_state.regs[2] = addr.value() as _;
    
    // 2. 设置 sepc 指向信号处理函数地址
    new_state.sepc = sa.action.value() as _;
    
    // 3. 设置 ra (x1) 指向 trampoline (sa_restorer)，当处理函数返回时跳转回这里执行 sigreturn
    new_state.regs[1] = sa.restorer.unwrap().value() as _;
    
    // 4. 设置第一个参数 a0 (x10) 为信号 ID
    new_state.regs[10] = id.user_id();

    Ok(new_state)
}

pub async fn do_signal_return() -> Result<TrapFrame> {
    let task = current_task();

    // 从当前 SP 获取 Signal Frame 的位置
    // regs[2] 是 sp
    let sig_frame_addr: TUA<RtSigFrame> =
        TUA::from_value(task.ctx.lock_save_irq().user().regs[2] as _);

    // 从用户栈恢复 Frame
    let sig_frame = copy_from_user(sig_frame_addr).await?;

    // 如果使用了 Signal Stack，尝试恢复
    if !sig_frame.alt_stack_prev_addr.is_null() {
        task.process
            .signals
            .lock_save_irq()
            .alt_stack
            .as_mut()
            .expect("Alt stack disappeared during use")
            .restore_alt_stack(sig_frame.alt_stack_prev_addr);
    }

    // 返回恢复后的用户上下文 (uctx)
    Ok(sig_frame.uctx)
}