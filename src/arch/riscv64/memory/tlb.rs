use core::arch::asm;
use libkernel::memory::address::VA;

/// 架构无关的 TLB 失效接口
pub trait TLBInvalidator {
    fn invalidate_page(&self, va: VA);
}

/// RISC-V 使用 sfence.vma 实现
#[derive(Clone, Debug)]
pub struct SfenceTlbInvalidator;

impl TLBInvalidator for SfenceTlbInvalidator {
    fn invalidate_page(&self, va: VA) {
        unsafe {
            // sfence.vma va, asid(0)
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

/// =======================
/// 关键：对齐上层通用接口
/// =======================

/// 所有地址空间 TLB flush（RISC-V 没 EL 概念）
pub type AllTlbInvalidator = SfenceTlbInvalidator;

/// 如果你想保留“语义别名”，可以继续提供
pub type AllEl1TlbInvalidator = SfenceTlbInvalidator;
pub type AllEl0TlbInvalidator = SfenceTlbInvalidator;
