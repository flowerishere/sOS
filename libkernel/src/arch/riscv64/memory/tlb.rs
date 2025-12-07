use core::arch::asm;
use crate::memory::address::VA;

/// A trait for invalidating TLB entries.
/// This trait matches the interface expected by MappingContext.
pub trait TlbInvalidator {
    fn invalidate_page(&self, va: VA);
    fn invalidate_all(&self);
}

/// TLB Invalidator that uses the RISC-V `sfence.vma` instruction.
pub struct SfenceTlbInvalidator;

impl SfenceTlbInvalidator {
    pub fn new() -> Self {
        Self
    }
}

impl TlbInvalidator for SfenceTlbInvalidator {
    fn invalidate_page(&self, va: VA) {
        unsafe {
            // sfence.vma vaddr, asid
            // x0 for asid means all ASIDs (or current depending on implementation, usually we supply x0)
            asm!("sfence.vma {}, x0", in(reg) va.value());
        }
    }

    fn invalidate_all(&self) {
        unsafe {
            asm!("sfence.vma x0, x0");
        }
    }
}

/// A dummy invalidator for early boot (before MMU is on).
pub struct NullTlbInvalidator;

impl TlbInvalidator for NullTlbInvalidator {
    fn invalidate_page(&self, _va: VA) {}
    fn invalidate_all(&self) {}
}