pub trait TLBInvalidator {
    fn invalidate_page(&self, _addr: crate::memory::address::VA) {
        unsafe { riscv::asm::sfence_vma_all() }; // Simplified, ideally flush specific VA
    }
}

pub struct RiscvTLBInvalidator;
impl TLBInvalidator for RiscvTLBInvalidator {}