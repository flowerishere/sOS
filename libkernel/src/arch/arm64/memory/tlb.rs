use crate::memory::address::VA;
pub trait TLBInvalidator {
    fn invalidate_page(&self, va: VA);
}

pub struct NullTlbInvalidator {}

impl TLBInvalidator for NullTlbInvalidator {
    fn invalidate_page(&self, _va: VA) {}
}
