use crate::memory::address::VA;
use core::arch::asm;

pub trait TLBInvalidator {
    fn invalidate_page(&self, va: VA);
}

pub struct NullTlbInvalidator;

impl TLBInvalidator for NullTlbInvalidator {
    fn invalidate_page(&self, _va: VA) {}
}

#[derive(Clone, Debug)]
pub struct AllTlbInvalidator;

impl TLBInvalidator for AllTlbInvalidator {
    fn invalidate_page(&self, va: VA) {
        unsafe {
            asm!("sfence.vma {}", in(reg) va.value());
        }
    }
}


impl Drop for AllTlbInvalidator {
    fn drop(&mut self) {
        unsafe {
            asm!("sfence.vma x0, x0");
        }
    }
}