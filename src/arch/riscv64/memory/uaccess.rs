use core::{
    arch::{asm, global_asm},
    future::Future,
    mem::transmute,
    pin::Pin,
    task::{Context, Poll},
};

use alloc::boxed::Box;
use libkernel::{
    error::{KernelError, Result},
    memory::address::UA,
};
use log::error;

// 引入汇编文件
global_asm!(include_str!("uaccess.s"));

type Fut = dyn Future<Output = Result<()>> + Send;

unsafe impl Send for Riscv64CopyFromUser {}
unsafe impl Send for Riscv64CopyToUser {}
unsafe impl Send for Riscv64CopyStrnFromUser {}

pub const UACESS_ABORT_DENIED: usize = 1;
pub const UACESS_ABORT_DEFERRED: usize = 2;

/// 通用的 uaccess 轮询逻辑
fn poll_uaccess<F>(
    deferred_fault: &mut Option<Pin<Box<Fut>>>,
    bytes_copied: &mut usize,
    cx: &mut Context<'_>,
    mut do_copy: F,
) -> Poll<Result<usize>>
where
    F: FnMut(usize) -> (usize, usize, usize, usize),
{
    loop {
        // 1. 如果有挂起的缺页处理 Future，先轮询它
        if let Some(mut fut) = deferred_fault.take() {
            match fut.as_mut().poll(cx) {
                Poll::Ready(Err(_)) => return Poll::Ready(Err(KernelError::Fault)),
                Poll::Ready(Ok(())) => {} // 缺页处理完成，继续拷贝
                Poll::Pending => {
                    *deferred_fault = Some(fut);
                    return Poll::Pending;
                }
            }
        }

        // 2. 执行汇编拷贝
        // status: a0, work_ptr: a1, bytes_copied_new: a2, work_vtable: a3
        // 注意：汇编中 a2 是 offset，这里对应 bytes_copied
        let (status, work_ptr, bytes_copied_new, work_vtable) = do_copy(*bytes_copied);

        match status {
            0 => return Poll::Ready(Ok(bytes_copied_new)), // 成功
            UACESS_ABORT_DENIED => return Poll::Ready(Err(KernelError::Fault)),
            UACESS_ABORT_DEFERRED => {
                *bytes_copied = bytes_copied_new;
                let ptr: *mut Fut =
                    unsafe { transmute((work_ptr as *mut (), work_vtable as *const ())) };
                // 将裸指针恢复为 Pin<Box<Fut>>
                *deferred_fault = Some(unsafe { Box::into_pin(Box::from_raw(ptr)) });
            }
            _ => {
                error!("Unknown exit status from fault handler: {status}");
                return Poll::Ready(Err(KernelError::Fault));
            }
        }
    }
}

pub struct Riscv64CopyFromUser {
    src: UA,
    dst: *const (),
    len: usize,
    bytes_copied: usize,
    deferred_fault: Option<Pin<Box<Fut>>>,
}

impl Riscv64CopyFromUser {
    pub fn new(src: UA, dst: *const (), len: usize) -> Self {
        Self {
            src,
            dst,
            len,
            bytes_copied: 0,
            deferred_fault: None,
        }
    }
}

impl Future for Riscv64CopyFromUser {
    type Output = Result<()>;

    fn poll(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Self::Output> {
        let this = unsafe { self.get_unchecked_mut() };

        poll_uaccess(
            &mut this.deferred_fault,
            &mut this.bytes_copied,
            cx,
            |mut bytes_copied| {
                let mut status: usize;
                let mut work_ptr: usize;
                let mut work_vtable: usize;

                unsafe {
                    asm!(
                        "call __do_copy_from_user",
                        in("a0") this.src.value(),
                        in("a1") this.dst,
                        inout("a2") bytes_copied, // Input: offset, Output: new offset
                        in("a3") this.len,
                        lateout("a0") status,
                        lateout("a1") work_ptr,
                        // lateout("a2") is bytes_copied
                        lateout("a3") work_vtable, // 借用 a3 返回 vtable
                        
                        // Clobbers: RISC-V call clobbers ra and temporaries
                        out("ra") _, out("t0") _, out("t1") _, out("t2") _
                    )
                }
                // Return order matches poll_uaccess signature expectance
                (status, work_ptr, bytes_copied, work_vtable)
            },
        )
        .map(|x| x.map(|_| ()))
    }
}

pub struct Riscv64CopyStrnFromUser {
    src: UA,
    dst: *mut u8,
    len: usize,
    bytes_copied: usize,
    deferred_fault: Option<Pin<Box<Fut>>>,
}

impl Riscv64CopyStrnFromUser {
    pub fn new(src: UA, dst: *mut u8, len: usize) -> Self {
        Self {
            src,
            dst,
            len,
            bytes_copied: 0,
            deferred_fault: None,
        }
    }
}

impl Future for Riscv64CopyStrnFromUser {
    type Output = Result<usize>;

    fn poll(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Self::Output> {
        let this = unsafe { self.get_unchecked_mut() };

        poll_uaccess(
            &mut this.deferred_fault,
            &mut this.bytes_copied,
            cx,
            |mut bytes_copied| {
                let mut status: usize;
                let mut work_ptr: usize;
                let mut work_vtable: usize;

                unsafe {
                    asm!(
                        "call __do_copy_from_user_halt_nul",
                        in("a0") this.src.value(),
                        in("a1") this.dst,
                        inout("a2") bytes_copied,
                        in("a3") this.len,
                        lateout("a0") status,
                        lateout("a1") work_ptr,
                        lateout("a3") work_vtable,
                        out("ra") _, out("t0") _, out("t1") _, out("t2") _
                    )
                }

                (status, work_ptr, bytes_copied, work_vtable)
            },
        )
    }
}

pub struct Riscv64CopyToUser {
    src: *const (),
    dst: UA,
    len: usize,
    bytes_copied: usize,
    deferred_fault: Option<Pin<Box<Fut>>>,
}

impl Riscv64CopyToUser {
    pub fn new(src: *const (), dst: UA, len: usize) -> Self {
        Self {
            src,
            dst,
            len,
            bytes_copied: 0,
            deferred_fault: None,
        }
    }
}

impl Future for Riscv64CopyToUser {
    type Output = Result<()>;

    fn poll(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Self::Output> {
        let this = unsafe { self.get_unchecked_mut() };

        poll_uaccess(
            &mut this.deferred_fault,
            &mut this.bytes_copied,
            cx,
            |mut bytes_copied| {
                let mut status: usize;
                let mut work_ptr: usize;
                let mut work_vtable: usize;

                unsafe {
                    asm!(
                        "call __do_copy_to_user",
                        in("a0") this.src,
                        in("a1") this.dst.value(),
                        inout("a2") bytes_copied,
                        in("a3") this.len,
                        lateout("a0") status,
                        lateout("a1") work_ptr,
                        lateout("a3") work_vtable,
                        out("ra") _, out("t0") _, out("t1") _, out("t2") _
                    )
                }
                (status, work_ptr, bytes_copied, work_vtable)
            },
        )
        .map(|x| x.map(|_| ()))
    }
}