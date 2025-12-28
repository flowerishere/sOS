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

const STATIC_PAGE_COUNT: usize = 128;
const MAX_FDT_SIZE: usize = 2 * 1024 * 1024;
const SATP_MODE_SV48: usize = 9;

const UART_BASE: u64 = 0x1000_0000;
const PLIC_BASE: u64 = 0x0c00_0000;
const CLINT_BASE: u64 = 0x0200_0000;

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

struct StaticPageAllocator {
    base: PA,
    allocated: usize,
}

impl StaticPageAllocator {
    fn from_phys_adr(addr: PA) -> Self {
        debug!("[ALLOC] Initializing at 0x");
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
        unsafe {
            ptr::write_bytes(ret.as_ptr_mut() as *mut u8, 0, PAGE_SIZE);
        }
        
        self.allocated += 1;
        debug!("[ALLOC] Page ");
        unsafe { print_hex(self.allocated); }
        debug!("/");
        unsafe { print_hex(STATIC_PAGE_COUNT); }
        debug!("\n");
        
        Ok(ret)
    }
}

struct KernelImageTranslator {}

impl<T> AddressTranslator<T> for KernelImageTranslator {
    fn virt_to_phys(_va: TVA<T>) -> TPA<T> {
        unreachable!()
    }
    fn phys_to_virt(_pa: TPA<T>) -> TVA<T> {
        IMAGE_BASE.cast()
    }
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

fn do_paging_bootstrap(static_pages: PA, image_addr: PA, fdt_addr: PA) -> Result<PA> {
    debug!("\n=== Paging Bootstrap Start ===\n");
    debug!("[INFO] static_pages = 0x");
    unsafe { print_hex(static_pages.value()); }
    debug!("\n[INFO] image_addr   = 0x");
    unsafe { print_hex(image_addr.value()); }
    debug!("\n[INFO] fdt_addr     = 0x");
    unsafe { print_hex(fdt_addr.value()); }
    debug!("\n");

    let mut bump_alloc = StaticPageAllocator::from_phys_adr(static_pages);

    debug!("[BOOT] Allocating root table...\n");
    let root_table_pa = bump_alloc.allocate_page_table::<L0Table>()?;
    debug!("[BOOT] Root table at 0x");
    unsafe { print_hex(root_table_pa.to_untyped().value()); }
    debug!("\n");

    let image_size = unsafe {
        let start = &__image_start as *const _ as usize;
        let end = &__image_end as *const _ as usize;
        debug!("[KERN] Image range: 0x");
        print_hex(start);
        debug!(" - 0x");
        print_hex(end);
        debug!("\n");
        
        if end <= start {
            debug!("[ERROR] Invalid image bounds\n");
            park_cpu();
        }
        end - start
    };

    let padding_size = 64 * 1024 * 1024; 
    let image_size_with_padding = image_size + padding_size;

    let image_size_aligned = (image_size_with_padding + PAGE_SIZE - 1) & !(PAGE_SIZE - 1);
    
    debug!("[KERN] Mapping size (with padding) = ");
    unsafe { print_hex(image_size_aligned); }
    debug!(" bytes\n");

    let kernel_range = PhysMemoryRegion::new(image_addr, image_size_aligned);

    let mut translator = IdmapTranslator {};
    let invalidator = AllTlbInvalidator {};
    let mut ctx = MappingContext {
        allocator: &mut bump_alloc,
        mapper: &mut translator,
        invalidator: &invalidator,
    };

    debug!("[MAP] Identity mapping kernel...\n");
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
    debug!("[MAP] Identity map OK\n");

    debug!("[MAP] High mapping kernel to IMAGE_BASE...\n");
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
    debug!("[MAP] High map OK\n");

    debug!("[MAP] Mapping UART...\n");
    let uart_range = PhysMemoryRegion::new(PA::from_value(UART_BASE as usize), PAGE_SIZE);
    map_range(
        root_table_pa,
        MapAttributes {
            phys: uart_range,
            virt: uart_range.map_via::<IdentityTranslator>(),
            mem_type: MemoryType::Normal, 
            perms: PtePermissions::rw(false),
        },
        &mut ctx,
    )?;
    debug!("[MAP] UART OK\n");

    debug!("[MAP] Mapping PLIC...\n");
    let plic_range = PhysMemoryRegion::new(PA::from_value(PLIC_BASE as usize), 0x400000);
    map_range(
        root_table_pa,
        MapAttributes {
            phys: plic_range,
            virt: plic_range.map_via::<IdentityTranslator>(),
            mem_type: MemoryType::Normal,
            perms: PtePermissions::rw(false),
        },
        &mut ctx,
    )?;
    debug!("[MAP] PLIC OK\n");

    debug!("[MAP] Mapping CLINT...\n");
    let clint_range = PhysMemoryRegion::new(PA::from_value(CLINT_BASE as usize), 0x10000);
    map_range(
        root_table_pa,
        MapAttributes {
            phys: clint_range,
            virt: clint_range.map_via::<IdentityTranslator>(),
            mem_type: MemoryType::Normal,
            perms: PtePermissions::rw(false),
        },
        &mut ctx,
    )?;
    debug!("[MAP] CLINT OK\n");

    debug!("[MAP] Mapping FDT...\n");
    let fdt_range = PhysMemoryRegion::new(fdt_addr, MAX_FDT_SIZE);
    map_range(
        root_table_pa,
        MapAttributes {
            phys: fdt_range,
            virt: fdt_range.map_via::<IdentityTranslator>(),
            mem_type: MemoryType::Normal,
            perms: PtePermissions::rw(false),
        },
        &mut ctx,
    )?;
    debug!("[MAP] FDT OK\n");

    debug!("[MMU] Enabling MMU...\n");
    enable_mmu(root_table_pa.to_untyped());
    
    
    Ok(root_table_pa.to_untyped())
}

#[unsafe(no_mangle)]
pub extern "C" fn enable_mmu(root_table_pa: PA) {
    let ppn = root_table_pa.value() >> 12;
    
    debug!("[MMU] Root PPN = 0x");
    unsafe { print_hex(ppn); }
    debug!("\n[MMU] SATP = 0x");
    
    let satp_value = (SATP_MODE_SV48 << 60) | ppn;
    unsafe { print_hex(satp_value); }
    debug!("\n[MMU] Enabling now...\n");
    
    
    if ppn > 0x8_0000_0000 {
        debug!("[ERROR] Invalid PPN\n");
        park_cpu();
    }

    unsafe {
        satp::write(satp_value);
        core::sync::atomic::fence(core::sync::atomic::Ordering::SeqCst);
        asm::sfence_vma_all();
        core::sync::atomic::fence(core::sync::atomic::Ordering::SeqCst);
    }
    
}

#[unsafe(no_mangle)]
pub extern "C" fn paging_bootstrap(static_pages: PA, image_addr: PA, fdt_addr: PA) -> PA {
    match do_paging_bootstrap(static_pages, image_addr, fdt_addr) {
        Ok(addr) => {
            debug!("[SUCCESS] Bootstrap complete\n\n");
            addr
        }
        Err(e) => {
            debug!("[FATAL] Bootstrap failed: ");
            match e {
                KernelError::NoMemory => debug!("NoMemory\n"),
                _ => debug!("Unknown error\n"),
            }
            park_cpu()
        }
    }
}

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