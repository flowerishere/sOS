use super::park_cpu;
use core::arch::asm;

#[inline(never)]
#[unsafe(no_mangle)]
pub extern "C" fn transition_to_sv_mode(stack_addr: u64) {
    unsafe {
        asm!("mv sp, {0}", in(reg) stack_addr);
    }
}