use libkernel::memory::address::{PA, VA};
use linked_list_allocator::LockedHeap;

// -------------------------------------------------------------------
// 模块声明
// -------------------------------------------------------------------

pub mod address_space;
pub mod fault;
pub mod fixmap;
pub mod mmu;
pub mod tlb;
pub mod uaccess;

// -------------------------------------------------------------------
// 内存布局常量 (RISC-V Sv48)
// -------------------------------------------------------------------

// 内核空间起始地址 (Sv48: 0xFFFF_8000_0000_0000)

pub const PAGE_OFFSET: usize = 0xffff_8000_0000_0000;

// 内核镜像链接基址
pub const IMAGE_BASE: VA = VA::from_value(0xffff_8000_0000_0000);

// Fixmap 区域基址 (用于临时映射、FDT 解析等)
pub const FIXMAP_BASE: VA = VA::from_value(0xffff_9000_0000_0000);

// MMIO 映射区域基址
pub const MMIO_BASE: VA = VA::from_value(0xffff_d000_0000_0000);

// -------------------------------------------------------------------
// 全局分配器与地址翻译
// -------------------------------------------------------------------

const BOGUS_START: PA = PA::from_value(usize::MAX);
static mut KIMAGE_START: PA = BOGUS_START;

#[global_allocator]
pub static HEAP_ALLOCATOR: LockedHeap = LockedHeap::empty();

/// 获取内核符号的物理地址
#[macro_export]
macro_rules! ksym_pa {
    ($sym:expr) => {{
        let v = libkernel::memory::address::VA::from_value(core::ptr::addr_of!($sym) as usize);
        $crate::arch::riscv64::memory::translate_kernel_va(v)
    }};
}

/// 获取内核函数的物理地址
#[macro_export]
macro_rules! kfunc_pa {
    ($sym:expr) => {{
        let v = libkernel::memory::address::VA::from_value($sym as usize);
        $crate::arch::riscv64::memory::translate_kernel_va(v)
    }};
}

/// 设置内核加载的物理起始地址 (通常由早期启动汇编或 setup 代码调用)
pub fn set_kimage_start(pa: PA) {
    unsafe {
        if KIMAGE_START != BOGUS_START {
            panic!("Attempted to change RAM_START, once set");
        }
        KIMAGE_START = pa;
    }
}

pub fn get_kimage_start() -> PA {
    unsafe {
        if KIMAGE_START == BOGUS_START {
            panic!("attempted to access RAM_START before being set");
        }
        KIMAGE_START
    }
}

/// 简单的线性偏移转换：Kernel VA -> PA
/// 仅适用于线性映射区域（如内核代码段）
pub fn translate_kernel_va(addr: VA) -> PA {
    let mut v = addr.value();
    // 减去虚拟基址，加上物理基址
    v -= IMAGE_BASE.value();
    PA::from_value(v + get_kimage_start().value())
}