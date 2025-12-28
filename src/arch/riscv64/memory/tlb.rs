use core::arch::asm;
use libkernel::memory::address::VA;

pub trait TLBInvalidator {
    fn invalidate_page(&self, va: VA);
}

#[derive(Clone, Debug)]
pub struct SfenceTlbInvalidator;

impl TLBInvalidator for SfenceTlbInvalidator {
    fn invalidate_page(&self, va: VA) {
        unsafe {
            asm!(
                "sfence.vma {va}, x0",
                va = in(reg) va.value(),
                options(nostack, preserves_flags),
            );
        }
    }
}

impl Drop for SfenceTlbInvalidator {
    fn drop(&mut self) {
        unsafe {
            // flush all
            asm!(
                "sfence.vma x0, x0",
                options(nostack, preserves_flags),
            );
        }
    }
}


