use crate::memory::address::VA;

#[derive(Clone, Debug)]
#[repr(C)]
pub struct TrapFrame {
    pub regs: [usize; 32],
    pub sstatus: usize,
    pub sepc: usize,
}

impl TrapFrame {
    pub fn new_user(entry: VA, stack: VA) -> Self {
        let mut tf = Self {
            regs: [0; 32],
            sstatus: 0,
            sepc: entry.value(),
        };
        tf.regs[2] = stack.value(); // sp
        tf
    }
}