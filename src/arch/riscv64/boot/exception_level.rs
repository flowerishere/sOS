use super::park_cpu;
use core::arch::asm;
use riscv::register::sstatus;

#[inline(never)]
#[no_mangle]
pub extern "C" fn transition_to_sv_mode(stack_addr: u64) {
    unsafe {
        // Clear SIE (Supervisor Interrupt Enable) bit in sstatus
        sstatus::clear_sie();
    }
    unsafe {
        asm!("mv sp, {0}", in(reg) stack_addr);
    }
}