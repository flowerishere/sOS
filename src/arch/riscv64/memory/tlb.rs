use core::arch::asm;
use libkernel::arch::riscv64::memory::tlb::TLBInvalidator;
use libkernel::memory::address::VA;
/// Trait definition (usually imported from common or defined here if specific)
/// Assuming TLBInvalidator is defined in common `libkernel` or we define the trait here.
/// Based on ARM code, it seems `TLBInvalidator` is a trait. 
/// If it's not in libkernel common, we define it:

pub struct NullTlbInvalidator;
impl TLBInvalidator for NullTlbInvalidator {
    fn invalidate_page(&self, _va: VA) {}
}

#[derive(Clone, Debug)]
pub struct AllTlbInvalidator;

impl AllTlbInvalidator {
    pub fn new() -> Self {
        Self
    }
}

impl Drop for AllTlbInvalidator {
    fn drop(&mut self) {
        unsafe {
            // Flush all TLB entries. 
            // sfence.vma x0, x0 flushes all address spaces and VAs.
            asm!("sfence.vma x0, x0"); 
        }
    }
}

// RISC-V usually doesn't distinguish EL1/EL0 flushes in the same way.
// We can use the same invalidator for both contexts or specifically flush ASIDs.
// For simplicity, we alias them.
pub type AllEl1TlbInvalidator = AllTlbInvalidator;
pub type AllEl0TlbInvalidator = AllTlbInvalidator;