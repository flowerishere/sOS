use core::arch::asm;
use crate::memory::address::VA;

/// A trait for invalidating TLB entries.
pub trait TlbInvalidator {
    fn invalidate_page(&self, va: VA);
    fn invalidate_all(&self);
}

/// RISC-V implementation using sfence.vma
pub struct SfenceTlbInvalidator;

impl SfenceTlbInvalidator {
    pub fn new() -> Self {
        Self
    }
}

impl TlbInvalidator for SfenceTlbInvalidator {
    fn invalidate_page(&self, va: VA) {
        unsafe {
            asm!("sfence.vma {}, x0", in(reg) va.value());
        }
    }

    fn invalidate_all(&self) {
        unsafe {
            asm!("sfence.vma x0, x0");
        }
    }
}

/// Dummy invalidator
pub struct NullTlbInvalidator;

impl TlbInvalidator for NullTlbInvalidator {
    fn invalidate_page(&self, _va: VA) {}
    fn invalidate_all(&self) {}
}