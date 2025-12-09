use crate::CpuOps;
use super::Riscv64;
use riscv::register::sstatus;

impl CpuOps for Riscv64 {
    #[inline(always)]
    fn id() -> usize {
        let hartid: usize;
        unsafe { core::arch::asm!("mv {}, tp", out(reg) hartid) };
        hartid
    }

    fn halt() -> ! {
        loop {
            unsafe { riscv::asm::wfi() };
        }
    }

    #[inline(always)]
    fn enable_interrupts() {
        unsafe { sstatus::set_sie() };
    }

    #[inline(always)]
    fn disable_interrupts() -> usize {
        let flags: usize;
        unsafe { 
            core::arch::asm!("csrr {}, sstatus", out(reg) flags);
            sstatus::clear_sie();
        }
        flags
    }

    #[inline(always)]
    fn restore_interrupt_state(flags: usize) {
        if (flags & (1 << 1)) != 0 {
            unsafe { sstatus::set_sie() };
        }
    }
}