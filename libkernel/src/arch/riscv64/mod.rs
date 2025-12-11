use crate::{
    CpuOps, VirtualMemory,
    error::{Result, KernelError},
    memory::address::{UA, VA},
    sync::spinlock::SpinLockIrq,
};
use core::future::Future;

pub mod memory;

use self::memory::mmu::{RiscvKernelAddressSpace, KERN_ADDR_SPACE, RiscvProcessAddressSpace};
use self::memory::pg_tables::RvPageTableRoot;

pub struct Riscv64;

impl CpuOps for Riscv64 {
    fn id() -> usize {
        let hartid: usize;
        #[cfg(any(target_arch = "riscv64", target_arch = "riscv32"))]
        unsafe { core::arch::asm!("mv {}, tp", out(reg) hartid) };
        
        #[cfg(not(any(target_arch = "riscv64", target_arch = "riscv32")))]
        { hartid = 0; }
        
        hartid
    }

    fn halt() -> ! {
        loop {
            #[cfg(any(target_arch = "riscv64", target_arch = "riscv32"))]
            unsafe { core::arch::asm!("wfi") };
        }
    }

    fn disable_interrupts() -> usize {
        let prev: usize;
        #[cfg(any(target_arch = "riscv64", target_arch = "riscv32"))]
        unsafe {
            core::arch::asm!("csrrci {}, sstatus, 2", out(reg) prev);
        }
        #[cfg(not(any(target_arch = "riscv64", target_arch = "riscv32")))]
        { prev = 0; }
        
        prev & 0x2
    }

    fn restore_interrupt_state(flags: usize) {
        if flags != 0 {
            #[cfg(any(target_arch = "riscv64", target_arch = "riscv32"))]
            unsafe { core::arch::asm!("csrrs x0, sstatus, 2") };
        }
    }

    fn enable_interrupts() {
        #[cfg(any(target_arch = "riscv64", target_arch = "riscv32"))]
        unsafe { core::arch::asm!("csrrs x0, sstatus, 2") };
    }
}

impl VirtualMemory for Riscv64 {
    type PageTableRoot = RvPageTableRoot;
    type ProcessAddressSpace = RiscvProcessAddressSpace;
    type KernelAddressSpace = RiscvKernelAddressSpace;

    const PAGE_OFFSET: usize = 0xffff_ffc0_0000_0000;

    fn kern_address_space() -> &'static SpinLockIrq<Self::KernelAddressSpace, Self> {
        &KERN_ADDR_SPACE
    }
}

#[derive(Clone, Debug)]
#[repr(C)]
pub struct TrapFrame {
    pub regs: [usize; 32],
    pub sstatus: usize,
    pub sepc: usize,
}

impl crate::arch::Arch for Riscv64 {
    type UserContext = TrapFrame;

    fn name() -> &'static str {
        "riscv64"
    }

    fn new_user_context(entry_point: VA, stack_top: VA) -> Self::UserContext {
        let mut ctx = TrapFrame {
            regs: [0; 32],
            sstatus: 0,
            sepc: entry_point.value(),
        };
        ctx.sstatus = 1 << 5; 
        ctx.regs[2] = stack_top.value();
        ctx
    }

    fn power_off() -> ! {
        #[cfg(feature = "arch-riscv64")]
        sbi_rt::system_reset(sbi_rt::Shutdown, sbi_rt::NoReason);
        Self::halt()
    }

    fn do_signal_return() -> impl Future<Output = Result<<Self as crate::arch::Arch>::UserContext>> {
        async { unimplemented!("do_signal_return") }
    }

    unsafe fn copy_from_user(src: UA, dst: *mut (), len: usize) -> impl Future<Output = Result<()>> {
        async move {
            unsafe {
                core::ptr::copy_nonoverlapping(src.value() as *const u8, dst as *mut u8, len);
            }
            Ok(())
        }
    }

    unsafe fn copy_to_user(src: *const (), dst: UA, len: usize) -> impl Future<Output = Result<()>> {
        async move {
            unsafe {
                core::ptr::copy_nonoverlapping(src as *const u8, dst.value() as *mut u8, len);
            }
            Ok(())
        }
    }

    unsafe fn copy_strn_from_user(src: UA, dst: *mut u8, len: usize) -> impl Future<Output = Result<usize>> {
        async move {
            let src_ptr = src.value() as *const u8;
            for i in 0..len {
                unsafe {
                    let val = *src_ptr.add(i);
                    *dst.add(i) = val;
                    if val == 0 {
                        return Ok(i);
                    }
                }
            }
            Err(KernelError::NameTooLong)
        }
    }
}