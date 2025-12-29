use core::ptr;
use libkernel::arch::riscv64::memory::pg_descriptors::MemoryType;
use libkernel::arch::riscv64::memory::pg_tables::{
    L0Table, MapAttributes, MappingContext, PageAllocator, PageTableMapper, PgTable, PgTableArray,
    map_range,
};
use libkernel::arch::riscv64::memory::tlb::AllTlbInvalidator;
use libkernel::error::{KernelError, Result};
use libkernel::memory::address::{AddressTranslator, IdentityTranslator, PA, TPA, TVA};
use libkernel::memory::permissions::PtePermissions;
use libkernel::memory::region::PhysMemoryRegion;
use libkernel::memory::{PAGE_MASK, PAGE_SIZE};
use riscv::asm;
use riscv::register::satp;

use crate::arch::riscv64::memory::IMAGE_BASE;
use super::park_cpu;

// 给分配器足够的空间
const STATIC_PAGE_COUNT: usize = 512; 
const MAX_FDT_SIZE: usize = 2 * 1024 * 1024;
const SATP_MODE_SV48: usize = 9;

const UART_BASE: u64 = 0x1000_0000;
const PLIC_BASE: u64 = 0x0c00_0000;
const CLINT_BASE: u64 = 0x0200_0000;

// [Image Symbols]
unsafe extern "C" {
    static __image_start: u8;
    static __image_end: u8;
}

#[inline(always)]
unsafe fn uart_putc(c: u8) {
    ptr::write_volatile(0x1000_0000 as *mut u8, c);
}

#[inline(always)]
unsafe fn uart_puts(s: &str) {
    for b in s.bytes() {
        uart_putc(b);
    }
}

macro_rules! debug {
    ($s:expr) => {
        unsafe { uart_puts($s); }
    };
}

// 简单的 Hex 打印工具
unsafe fn print_hex(mut val: usize) {
    let hex_chars = b"0123456789abcdef";
    let mut buf = [0u8; 16];
    let mut i = 0;
    
    if val == 0 {
        uart_putc(b'0');
        return;
    }
    while val > 0 {
        buf[i] = hex_chars[val & 0xf];
        val >>= 4;
        i += 1;
    }
    while i > 0 {
        i -= 1;
        uart_putc(buf[i]);
    }
}

struct StaticPageAllocator {
    base: PA,
    allocated: usize,
}

impl StaticPageAllocator {
    fn from_phys_adr(addr: PA) -> Self {
        debug!("[ALLOC] Init at Safe Address: 0x");
        unsafe { print_hex(addr.value()); }
        debug!("\n");
        
        if addr.value() & PAGE_MASK != 0 {
            debug!("[ERROR] Unaligned allocator base!\n");
            park_cpu();
        }
        Self { base: addr, allocated: 0 }
    }
}

impl PageAllocator for StaticPageAllocator {
    fn allocate_page_table<T: PgTable>(&mut self) -> Result<TPA<PgTableArray<T>>> {
        if self.allocated >= STATIC_PAGE_COUNT {
            debug!("[ERROR] Out of pages\n");
            return Err(KernelError::NoMemory);
        }
        
        let ret: TPA<PgTableArray<T>> = TPA::from_value(self.base.add_pages(self.allocated).value());
        
        // 安全清零：现在 base 位于 image_end + 2MB，绝对安全
        unsafe {
            ptr::write_bytes(ret.as_ptr_mut() as *mut u8, 0, PAGE_SIZE);
        }
        
        self.allocated += 1;
        Ok(ret)
    }
}

struct KernelImageTranslator {}
impl<T> AddressTranslator<T> for KernelImageTranslator {
    fn virt_to_phys(_va: TVA<T>) -> TPA<T> { unreachable!() }
    fn phys_to_virt(_pa: TPA<T>) -> TVA<T> { IMAGE_BASE.cast() }
}

struct IdmapTranslator {}
impl PageTableMapper for IdmapTranslator {
    unsafe fn with_page_table<T: PgTable, R>(
        &mut self,
        pa: TPA<PgTableArray<T>>,
        f: impl FnOnce(TVA<PgTableArray<T>>) -> R
    ) -> Result<R> {
        let va = TVA::from_value(pa.value());
        Ok(f(va))
    }
}

fn do_paging_bootstrap(_bad_static_pages: PA, image_addr: PA, fdt_addr: PA) -> Result<PA> {
    debug!("\n=== Paging Bootstrap (Safety Offset Mode) ===\n");
    
    let image_start = unsafe { &__image_start as *const _ as usize };
    let image_end = unsafe { &__image_end as *const _ as usize };
    
    debug!("[KERN] Image: 0x"); unsafe { print_hex(image_start); }
    debug!(" - 0x"); unsafe { print_hex(image_end); }
    debug!("\n");

    let image_size = image_end - image_start;

    // =========================================================================
    // [Method 2 Implementation] 
    // 强制偏移 2MB (0x200000) 以避开内核镜像和任何潜在的 footer/padding
    // =========================================================================
    let image_end_aligned = (image_end + PAGE_SIZE - 1) & !(PAGE_SIZE - 1);
    let offset_2mb = 0x200_000; 
    let safe_alloc_base_val = image_end_aligned + offset_2mb;
    let safe_alloc_base = PA::from_value(safe_alloc_base_val);

    debug!("[FIX] Allocator Base = Image End + 2MB = 0x");
    unsafe { print_hex(safe_alloc_base_val); }
    debug!("\n");

    let mut bump_alloc = StaticPageAllocator::from_phys_adr(safe_alloc_base);

    // 预留足够大的 Padding (64MB) 确保 Allocator 也在 Identity Map 范围内
    // Image Start | ... Image ... | ... 2MB Gap ... | Allocator | ... Remaining Padding ...
    let padding_size = 64 * 1024 * 1024; 
    let total_map_size = image_size + offset_2mb + (STATIC_PAGE_COUNT * PAGE_SIZE) + padding_size;
    
    // 对齐映射大小
    let map_size_aligned = (total_map_size + 0x200000 - 1) & !(0x200000 - 1); // 2MB 对齐

    debug!("[KERN] Total Mapped Region: ");
    unsafe { print_hex(map_size_aligned); }
    debug!(" bytes\n");

    let kernel_range = PhysMemoryRegion::new(image_addr, map_size_aligned);

    debug!("[BOOT] Allocating root table...\n");
    let root_table_pa = bump_alloc.allocate_page_table::<L0Table>()?;
    debug!("[BOOT] Root table PA: 0x");
    unsafe { print_hex(root_table_pa.to_untyped().value()); }
    debug!("\n");

    let mut translator = IdmapTranslator {};
    let invalidator = AllTlbInvalidator {};
    let mut ctx = MappingContext {
        allocator: &mut bump_alloc,
        mapper: &mut translator,
        invalidator: &invalidator,
    };

    // 1. Identity Mapping (覆盖 Kernel + 2MB Gap + Allocator)
    debug!("[MAP] Identity Mapping...\n");
    map_range(
        root_table_pa,
        MapAttributes {
            phys: kernel_range,
            virt: kernel_range.map_via::<IdentityTranslator>(),
            mem_type: MemoryType::Normal,
            perms: PtePermissions::rwx(false),
        },
        &mut ctx,
    )?;

    // 2. High Mapping (覆盖 Kernel + 2MB Gap + Allocator)
    debug!("[MAP] High Mapping...\n");
    map_range(
        root_table_pa,
        MapAttributes {
            phys: kernel_range,
            virt: kernel_range.map_via::<KernelImageTranslator>(),
            mem_type: MemoryType::Normal,
            perms: PtePermissions::rwx(false),
        },
        &mut ctx,
    )?;

    // 3. Devices
    debug!("[MAP] Mapping Devices...\n");
    let uart_range = PhysMemoryRegion::new(PA::from_value(UART_BASE as usize), PAGE_SIZE);
    map_range(root_table_pa, MapAttributes {
        phys: uart_range, virt: uart_range.map_via::<IdentityTranslator>(),
        mem_type: MemoryType::Normal, perms: PtePermissions::rw(false),
    }, &mut ctx)?;

    let plic_range = PhysMemoryRegion::new(PA::from_value(PLIC_BASE as usize), 0x400000);
    map_range(root_table_pa, MapAttributes {
        phys: plic_range, virt: plic_range.map_via::<IdentityTranslator>(),
        mem_type: MemoryType::Normal, perms: PtePermissions::rw(false),
    }, &mut ctx)?;

    let clint_range = PhysMemoryRegion::new(PA::from_value(CLINT_BASE as usize), 0x10000);
    map_range(root_table_pa, MapAttributes {
        phys: clint_range, virt: clint_range.map_via::<IdentityTranslator>(),
        mem_type: MemoryType::Normal, perms: PtePermissions::rw(false),
    }, &mut ctx)?;

    let fdt_range = PhysMemoryRegion::new(fdt_addr, MAX_FDT_SIZE);
    map_range(root_table_pa, MapAttributes {
        phys: fdt_range, virt: fdt_range.map_via::<IdentityTranslator>(),
        mem_type: MemoryType::Normal, perms: PtePermissions::rw(false),
    }, &mut ctx)?;

    debug!("[MMU] Enabling MMU...\n");
    enable_mmu(root_table_pa.to_untyped());
    
    Ok(root_table_pa.to_untyped())
}

#[unsafe(no_mangle)]
pub extern "C" fn enable_mmu(root_table_pa: PA) {
    let ppn = root_table_pa.value() >> 12;
    let satp_value = (SATP_MODE_SV48 << 60) | ppn;
    
    unsafe {
        satp::write(satp_value);
        asm::sfence_vma_all();
    }
}

#[unsafe(no_mangle)]
pub extern "C" fn paging_bootstrap(static_pages: PA, image_addr: PA, fdt_addr: PA) -> PA {
    match do_paging_bootstrap(static_pages, image_addr, fdt_addr) {
        Ok(addr) => {
            debug!("[SUCCESS] Bootstrap complete\n\n");
            addr
        }
        Err(_) => {
            debug!("[FATAL] Bootstrap failed\n");
            park_cpu()
        }
    }
}