use crate::memory::PAGE_ALLOC;
use super::{
    mmu::{page_allocator::PageTableAllocator, page_mapper::PageOffsetPgTableMapper, KERN_ADDR_SPACE},
    tlb::AllEl0TlbInvalidator,
};
use alloc::vec::Vec;
use libkernel::{
    PageInfo, UserAddressSpace,
    arch::riscv64::memory::{
        pg_descriptors::{L3Descriptor, MemoryType, PaMapper, PageTableEntry},
        pg_tables::{
            RvPageTableRoot, MapAttributes, MappingContext, PageAllocator, PgTableArray, map_range, PgTable,
        },
        pg_walk::{WalkContext, get_pte, walk_and_modify_region},
    },
    error::{KernelError, MapError, Result},
    memory::{
        PAGE_SIZE,
        address::{TPA, VA, TVA},
        page::PageFrame,
        permissions::PtePermissions,
        region::{PhysMemoryRegion, VirtMemoryRegion},
        pg_offset::PageOffsetTranslator,
    },
};
use riscv::register::satp;

pub struct RiscvProcessAddressSpace {
    l0_table: TPA<PgTableArray<RvPageTableRoot>>,
}

unsafe impl Send for RiscvProcessAddressSpace {}
unsafe impl Sync for RiscvProcessAddressSpace {}

impl UserAddressSpace for RiscvProcessAddressSpace {
    fn new() -> Result<Self>
    where
        Self: Sized,
    {
        // 1. Allocate a new empty L0 table
        let l0_table = PageTableAllocator::new().allocate_page_table::<RvPageTableRoot>()?;

        // 2. CRITICAL FOR RISC-V: Copy Kernel Mappings.
        // Since SATP handles both user and kernel space, we must copy the upper half 
        // mappings from the kernel page table to this new process page table.
        // Sv48: Kernel is at 0xFFFF_8000_..., which corresponds to the higher indices of L0.
        // We assume KERN_ADDR_SPACE is initialized.
        if let Some(kern_lock) = KERN_ADDR_SPACE.get() {
            let kern_as = kern_lock.lock_save_irq();
            let kern_l0_pa = kern_as.table_pa();
            
            unsafe {
                let kern_l0_ptr = kern_l0_pa
                    .cast::<u64>() 
                    .to_va::<PageOffsetTranslator<Sv48>>() 
                    .as_ptr();
                
                let user_l0_ptr = l0_table
                    .cast::<u64>()
                    .to_va::<PageOffsetTranslator<Sv48>>()
                    .as_ptr_mut();

                // In Sv48 (4 levels, 512 entries), the address space is split in half.
                // Lower half (0x0...0x7F...) is user, Upper half (0x80...0xFF...) is kernel.
                // The split happens exactly at index 256.
                // We copy entries 256 to 511.
                let start_idx = 256;
                let count = 256;
                
                core::ptr::copy_nonoverlapping(
                    kern_l0_ptr.add(start_idx), 
                    user_l0_ptr.add(start_idx), 
                    count
                );
            }
        }

        Ok(Self { l0_table })
    }

    fn activate(&self) {
        // Switch SATP to this process's page table.
        // Mode 9 = Sv48. ASID = 0 (for simplicity now, or manage ASIDs later).
        let ppn = self.l0_table.value() >> 12;
        unsafe {
            satp::set(satp::Mode::Sv48, 0, ppn);
            // Flush TLB to apply new address space
            riscv::asm::sfence_vma_all(); 
        }
    }

    fn deactivate(&self) {
        // Switch back to Kernel-only page table (usually Idle thread's table).
        // Or simply do nothing if the scheduler immediately switches to another task.
        // But for correctness, let's load the kernel root.
        if let Some(kern_lock) = KERN_ADDR_SPACE.get() {
            let kern_as = kern_lock.lock_save_irq();
            let ppn = kern_as.table_pa().value() >> 12;
            unsafe {
                satp::set(satp::Mode::Sv48, 0, ppn);
                riscv::asm::sfence_vma_all();
            }
        }
    }

    fn map_page(&mut self, page: PageFrame, va: VA, perms: PtePermissions) -> Result<()> {
        let mut ctx = MappingContext {
            allocator: &mut PageTableAllocator::new(),
            mapper: &mut PageOffsetPgTableMapper {},
            invalidator: &AllEl0TlbInvalidator::new(),
        };

        map_range(
            self.l0_table,
            MapAttributes {
                phys: page.as_phys_range(),
                virt: VirtMemoryRegion::new(va, PAGE_SIZE),
                mem_type: MemoryType::Normal,
                perms,
            },
            &mut ctx,
        )
    }

    fn unmap(&mut self, _va: VA) -> Result<PageFrame> {
        todo!("unmap single page")
    }

    fn protect_range(&mut self, va_range: VirtMemoryRegion, perms: PtePermissions) -> Result<()> {
        let mut walk_ctx = WalkContext {
            mapper: &mut PageOffsetPgTableMapper {},
            invalidator: &AllEl0TlbInvalidator::new(),
        };

        walk_and_modify_region(self.l0_table, va_range, &mut walk_ctx, |_, desc| {
            // Sv48 PTEs usually don't support explicit "swapped" bits in hardware,
            // but we use software defined bits.
            match (perms.is_execute(), perms.is_read(), perms.is_write()) {
                (false, false, false) => desc.mark_as_swapped(),
                _ => desc.set_permissions(perms),
            }
        })
    }

    fn unmap_range(&mut self, va_range: VirtMemoryRegion) -> Result<Vec<PageFrame>> {
        let mut walk_ctx = WalkContext {
            mapper: &mut PageOffsetPgTableMapper {},
            invalidator: &AllEl0TlbInvalidator::new(),
        };
        let mut claimed_pages = Vec::new();

        walk_and_modify_region(self.l0_table, va_range, &mut walk_ctx, |_, desc| {
            if let Some(addr) = desc.mapped_address() {
                claimed_pages.push(addr.to_pfn());
            }
            L3Descriptor::invalid()
        })?;

        Ok(claimed_pages)
    }

    fn remap(&mut self, va: VA, new_page: PageFrame, perms: PtePermissions) -> Result<PageFrame> {
        let mut walk_ctx = WalkContext {
            mapper: &mut PageOffsetPgTableMapper {},
            invalidator: &AllEl0TlbInvalidator::new(),
        };

        let mut old_pte = None;

        walk_and_modify_region(self.l0_table, va.page_region(), &mut walk_ctx, |_, pte| {
            old_pte = Some(pte);
            L3Descriptor::new_map_pa(new_page.pa(), MemoryType::Normal, perms)
        })?;

        old_pte
            .and_then(|pte| pte.mapped_address())
            .map(|a| a.to_pfn())
            .ok_or(KernelError::MappingError(MapError::NotL3Mapped))
    }

    fn translate(&self, va: VA) -> Option<PageInfo> {
        let pte = get_pte(
            self.l0_table,
            va.page_aligned(),
            &mut PageOffsetPgTableMapper {},
        )
        .unwrap()?;

        Some(PageInfo {
            pfn: pte.mapped_address()?.to_pfn(),
            perms: pte.permissions()?,
        })
    }

    fn protect_and_clone_region(
        &mut self,
        region: VirtMemoryRegion,
        other: &mut Self,
        new_perms: PtePermissions,
    ) -> Result<()>
    where
        Self: Sized,
    {
        let mut walk_ctx = WalkContext {
            mapper: &mut PageOffsetPgTableMapper {},
            invalidator: &AllEl0TlbInvalidator::new(),
        };

        walk_and_modify_region(self.l0_table, region, &mut walk_ctx, |va, pgd| {
            if let Some(addr) = pgd.mapped_address() {
                let page_region = PhysMemoryRegion::new(addr, PAGE_SIZE);
                let alloc1 = unsafe { PAGE_ALLOC.get().unwrap().alloc_from_region(page_region) };
                
                // Ref count logic
                alloc1.clone().leak();
                alloc1.leak();

                let mut ctx = MappingContext {
                    allocator: &mut PageTableAllocator::new(),
                    mapper: &mut PageOffsetPgTableMapper {},
                    invalidator: &AllEl0TlbInvalidator::new(),
                };

                map_range(
                    other.l0_table,
                    MapAttributes {
                        phys: PhysMemoryRegion::new(addr, PAGE_SIZE),
                        virt: VirtMemoryRegion::new(va, PAGE_SIZE),
                        mem_type: MemoryType::Normal,
                        perms: new_perms,
                    },
                    &mut ctx,
                )
                .unwrap();

                pgd.set_permissions(new_perms)
            } else {
                pgd
            }
        })
    }
}