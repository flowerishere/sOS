pub trait TLBInvalidator {}

pub struct NullTlbInvalidator {}

impl TLBInvalidator for NullTlbInvalidator {}
