use crate::{
    KernAddressSpace, UserAddressSpace, VirtualMemory,
    sync::spinlock::SpinLockIrq,
    memory::{
        address::{VA, PA}, 
        page::PageFrame,
        permissions::PtePermissions,
        region::{PhysMemoryRegion, VirtMemoryRegion}
    },
    error::Result,
    arch::riscv64::Riscv64,
};

pub static KERN_ADDR_SPACE: SpinLockIrq<RiscvKernelAddressSpace, Riscv64> = 
    SpinLockIrq::new(RiscvKernelAddressSpace { root_pa: 0 });

pub struct RiscvKernelAddressSpace {
    root_pa: usize,
}

impl KernAddressSpace for RiscvKernelAddressSpace {
    fn map_mmio(&mut self, region: PhysMemoryRegion) -> Result<VA> {
        Ok(VA::from_value(region.start_address().value() + Riscv64::PAGE_OFFSET))
    }

    fn map_normal(&mut self, _p: PhysMemoryRegion, _v: VirtMemoryRegion, _pe: PtePermissions) -> Result<()> {
        Ok(())
    }
}

pub struct RiscvProcessAddressSpace;
impl UserAddressSpace for RiscvProcessAddressSpace {
    fn new() -> Result<Self> { Ok(Self) }
    fn activate(&self) {}
    fn deactivate(&self) {}
    fn map_page(&mut self, _page: PageFrame, _va: VA, _perms: PtePermissions) -> Result<()> { Ok(()) }
    fn unmap(&mut self, _va: VA) -> Result<PageFrame> { Err(crate::error::KernelError::NotImplemented) }
    fn remap(&mut self, _va: VA, _n: PageFrame, _p: PtePermissions) -> Result<PageFrame> { Err(crate::error::KernelError::NotImplemented) }
    fn protect_range(&mut self, _v: VirtMemoryRegion, _p: PtePermissions) -> Result<()> { Ok(()) }
    fn unmap_range(&mut self, _v: VirtMemoryRegion) -> Result<alloc::vec::Vec<PageFrame>> { Ok(alloc::vec::Vec::new()) }
    fn translate(&self, _va: VA) -> Option<crate::PageInfo> { None }
    fn protect_and_clone_region(&mut self, _r: VirtMemoryRegion, _o: &mut Self, _p: PtePermissions) -> Result<()> { Ok(()) }
}