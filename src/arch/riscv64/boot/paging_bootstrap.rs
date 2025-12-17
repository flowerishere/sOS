use core::ptr;

use libkernel::arch::riscv64::memory::pg_descriptors::MemoryType;
use libkernel::arch::riscv64::memory::pg_tables::{
    L0Table, MapAttributes, MappingContext, PageAllocator, PageTableMapper, PgTable, PgTableArray,
    map_range,
};
use libkernel::arch::riscv64::memory::tlb::SfenceTlbInvalidator;
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

unsafe extern "C" {
    static __image_start: u8;
    static __image_end: u8;
}

struct StaticPageAllocator {
    base: PA,
    allocated: usize,
}

impl StaticPageAllocator {
    fn from_phys_adr(addr: PA) -> Self {
        if addr.value() & PAGE_MASK != 0 {
            park_cpu();
        }

        Self {
            base: addr,
            allocated: 0,
        }
    }

    fn peek<T>(&self) -> TPA<T> {
        TPA::from_value(self.base.add_pages(self.allocated).value())
    }
}

impl PageAllocator for StaticPageAllocator {
    fn allocate_page_table<T: PgTable>(&mut self) -> Result<TPA<PgTableArray<T>>> {
        if self.allocated == STATIC_PAGE_COUNT {
            return Err(KernelError::NoMemory);
        }

        let ret = self.peek::<PgTableArray<T>>();

        unsafe {
            ptr::write_bytes(ret.as_ptr_mut().cast::<u8>(), 0, PAGE_SIZE);
        }

        self.allocated += 1;

        Ok(ret)
    }
}

struct KernelImageTranslator {}

impl<T> AddressTranslator<T> for KernelImageTranslator {
    fn virt_to_phys(_va: libkernel::memory::address::TVA<T>) -> TPA<T> {
        unreachable!("Should only be used to translate PA -> VA")
    }

    fn phys_to_virt(_pa: TPA<T>) -> libkernel::memory::address::TVA<T> {
        IMAGE_BASE.cast()
    }
}

struct IdmapTranslator {}

impl PageTableMapper for IdmapTranslator {
    unsafe fn with_page_table<T: PgTable, R>(
        &mut self,
        pa: TPA<PgTableArray<T>>,
        f: impl FnOnce(TVA<PgTableArray<T>>) -> R,
    ) -> Result<R> {
        let va = KernelImageTranslator::phys_to_virt(pa);
        Ok(f(va))
    }
}

fn do_paging_bootstrap(static_pages: PA, image_addr:PA, fdt_addr: PA) -> Result<PA> {
    let mut bump_alloc = StaticPageAllocator::from_phys_adr(static_pages);
    //SAFETY:The MMU is currently disabled (or we are in early boot),
    //accesses to physical ram are unrestricted
    //RISC-V uses a single root table for both kernel and identity mappings
    let root_table_pa = bump_alloc.allocate_page_table::<L0Table>()?;
    //IDMAP kernel image
    let image_size = unsafe {
        (__image_end.as_ptr() as usize) - (__image_start.as_ptr() as usize)
    };
    let kernel_range = PhysMemoryRegion::new(image_addr, image_size);

    let mut translator = IdmapTranslator {};
    //RISC-V sfence.vma based invalidator usually not needed before MMU enabled
    //a Null one
    let invalidator = NullTlbInvalidator {};

    let mut bootstrap_ctx = MappingContext {
        allocator: &mut bump_alloc,
        mapper: &mut translator,
        invalidator: &invalidator,
    };
    
    //Identity Map
    //map kernel range identically so PC does't fault immediately after enabling MMU
    map_range(
        root_table_pa,
        MapAttributes {
            phys: kernel_range,
            virt: kernel_range.map_via::<IdentityTranslator>(),
            mem_type: MemoryType::Normal,
            perms: PtePermissions::rx(false),
        },
        &mut bootstrap_ctx,
    )?;

    //High Memory Map(Kernel Image)
    //Map the same physical kernel image to high virtual address
    map_range(
        root_table_pa,
        MapAttributes {
            phys: kernel_range,
            virt: kernel_range.map_via::<KernelImageTranslator>(),
            mem_type: MemoryType::Normal,
            perms: PtePermissions::rwx(false),
        },
        &mut bootstrap_ctx,
    )?;

    //Enable MMU logic
    enable_mmu(root_table_pa);

    Ok(root_table.to_untyped())
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
pub extern "C" fn paging_bootstrap(static_pages: PA, image_addr: PA, fdt_addr: PA,) -> PA {
    let res = do_paging_bootstrap(static_pages, image_phys_addr, fdt_addr);

    if let Ok(addr) = res {
        addr
    } else {
        park_cpu()
    }
}