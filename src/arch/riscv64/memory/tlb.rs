use core::arch::asm;
use libkernel::arch::riscv64::memory::tlb::TLBInvalidator;
use libkernel::memory::address::VA;
#[derive(Clone, Debug)]
pub struct SfenceTlbInvalidator;

impl TLBInvalidator for SfenceTlbInvalidator {
    fn invalidate_page(&self, va: VA) {
        unsafe {
            asm!("sfence.vma {}", in(reg) va.value());
        }
    }
}

impl Drop for SfenceTlbInvalidator {
    fn drop(&mut self) {
        unsafe {
            asm!("sfence.vma x0, x0");
        }
    }
}

pub type AllEl1TlbInvalidator = SfenceTlbInvalidator;
pub type AllEl0TlbInvalidator = SfenceTlbInvalidator;