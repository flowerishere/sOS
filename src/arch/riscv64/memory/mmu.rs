use super::{MMIO_BASE, tlb::AllEl1TlbInvalidator};
use crate::sync::{OnceLock, SpinLock};
use libkernel::{
    KernAddressSpace,
    arch::riscv64::memory::{
        pg_descriptors::{MemoryType, PaMapper},
        pg_tables::{MapAttributes, MappingContext, PgTableArray, RvPageTableRoot, map_range},
        pg_walk::get_pte,
        tlb::{AllTlbInvalidator, TLBInvalidator},
    },
    error::Result,
    memory::{
        address::{PA, TPA, VA},
        permissions::PtePermissions,
        region::{PhysMemoryRegion, VirtMemoryRegion},
    },
};

pub mod page_allocator;
pub mod page_mapper;
pub mod smalloc_page_allocator;

use self::page_allocator::PageTableAllocator;
use self::page_mapper::PageOffsetPgTableMapper;

pub static KERN_ADDR_SPACE: OnceLock<SpinLock<RiscvKernelAddressSpace>> = OnceLock::new();

pub struct RiscvKernelAddressSpace {
    kernel_l0: TPA<PgTableArray<RvPageTableRoot>>,
    mmio_ptr: VA,
}

impl RiscvKernelAddressSpace {
    fn do_map(&self, map_attrs: MapAttributes) -> Result<()> {
        let mut ctx = MappingContext {
            allocator: &mut PageTableAllocator::new(),
            mapper: &mut PageOffsetPgTableMapper {},
            invalidator: &AllTlbInvalidator {},
        };

        map_range(self.kernel_l0, map_attrs, &mut ctx)
    }

    pub fn translate(&self, va: VA) -> Option<PA> {
        let pg_offset = va.page_offset();
        let pte = get_pte(self.kernel_l0, va, &mut PageOffsetPgTableMapper {})
            .ok()
            .flatten()?;
        let pa = pte.mapped_address()?;
        Some(pa.add_bytes(pg_offset))
    }

    pub fn table_pa(&self) -> PA {
        self.kernel_l0.to_untyped()
    }
}

unsafe impl Send for RiscvKernelAddressSpace {}

impl KernAddressSpace for RiscvKernelAddressSpace {
    fn map_normal(
        &mut self,
        phys_range: PhysMemoryRegion,
        virt_range: VirtMemoryRegion,
        perms: PtePermissions,
    ) -> Result<()> {
        self.do_map(MapAttributes {
            phys: phys_range,
            virt: virt_range,
            mem_type: MemoryType::Normal,
            perms,
        })
    }

    fn map_mmio(&mut self, phys_range: PhysMemoryRegion) -> Result<VA> {
        let phys_mappable_region = phys_range.to_mappable_region();
        let base_va = self.mmio_ptr;
        let virt_range = VirtMemoryRegion::new(base_va, phys_mappable_region.region().size());

        self.do_map(MapAttributes {
            phys: phys_mappable_region.region(),
            virt: virt_range,
            mem_type: MemoryType::Device,
            perms: PtePermissions::rw(false),
        })?;

        self.mmio_ptr = VA::from_value(self.mmio_ptr.value() + phys_mappable_region.region().size());

        Ok(VA::from_value(base_va.value() + phys_mappable_region.offset()))
    }
}

pub fn setup_kern_addr_space(pa: TPA<PgTableArray<RvPageTableRoot>>) -> Result<()> {
    let addr_space = SpinLock::new(RiscvKernelAddressSpace {
        kernel_l0: pa,
        mmio_ptr: MMIO_BASE,
    });

    KERN_ADDR_SPACE
        .set(addr_space)
        .map_err(|_| libkernel::error::KernelError::InUse)
}