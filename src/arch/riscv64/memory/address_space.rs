use crate::memory::PAGE_ALLOC;
use super::{
    mmu::{page_allocator::PageTableAllocator, page_mapper::PageOffsetPgTableMapper, KERN_ADDR_SPACE},
};
use alloc::vec::Vec;
use libkernel::{
    PageInfo, UserAddressSpace,
    arch::riscv64::memory::{
        pg_descriptors::{L3Descriptor, MemoryType, PaMapper, PageTableEntry},
        pg_tables::{
            RvPageTableRoot, MapAttributes, MappingContext, PageAllocator, PgTableArray, map_range, PgTable
        },
        pg_walk::{WalkContext, get_pte, walk_and_modify_region},
        tlb::AllTlbInvalidator,
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
use crate::arch::ArchImpl;
pub struct RiscvProcessAddressSpace {
    // 使用 RvPageTableRoot (即 L0Table)
    l0_table: TPA<PgTableArray<RvPageTableRoot>>,
}

unsafe impl Send for RiscvProcessAddressSpace {}
unsafe impl Sync for RiscvProcessAddressSpace {}

impl UserAddressSpace for RiscvProcessAddressSpace {
    fn new() -> Result<Self>
    where
        Self: Sized,
    {
        // 1. 分配一个新的空 L0 表
        let l0_table = PageTableAllocator::new().allocate_page_table::<RvPageTableRoot>()?;

        // 2. RISC-V 关键步骤：复制内核映射
        // Sv48 模式下，内核位于高地址 (0xFFFF_8000_...)，对应 L0 表的高索引部分。
        // L0 表有 512 个条目，用户空间是 0-255，内核空间是 256-511。
        if let Some(kern_lock) = KERN_ADDR_SPACE.get() {
            let kern_as = kern_lock.lock_save_irq();
            let kern_l0_pa = kern_as.table_pa();
            
            unsafe {
                // 将物理地址转换为虚拟地址以便 CPU 访问进行 memcpy
                let kern_l0_ptr = kern_l0_pa
                    .cast::<u64>() 
                    .to_va::<PageOffsetTranslator<ArchImpl>>() 
                    .as_ptr();
                
                let user_l0_ptr = l0_table
                    .to_untyped()
                    .cast::<u64>()
                    .to_va::<PageOffsetTranslator<ArchImpl>>()
                    .as_ptr_mut();

                // 复制后半部分 (内核空间)
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
        // 切换 SATP 到当前进程的页表
        // Mode::Sv48 来自 riscv crate，而不是 pg_tables
        let ppn = self.l0_table.value() >> 12;
        unsafe {
            satp::set(satp::Mode::Sv48, 0, ppn);
            // 刷新 TLB
            riscv::asm::sfence_vma_all(); 
        }
    }

    fn deactivate(&self) {
        // 切换回内核页表 (通常是 Idle 线程的页表)
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
            invalidator: &AllTlbInvalidator{},
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
            invalidator: &AllTlbInvalidator{},
        };

        walk_and_modify_region(self.l0_table, va_range, &mut walk_ctx, |_, desc| {
            // Sv48 PTE 通常硬件不支持显式的 "swapped" 位
            // 这里利用软件定义位来标记
            match (perms.is_execute(), perms.is_read(), perms.is_write()) {
                (false, false, false) => desc.mark_as_swapped(),
                _ => desc.set_permissions(perms),
            }
        })
    }

    fn unmap_range(&mut self, va_range: VirtMemoryRegion) -> Result<Vec<PageFrame>> {
        let mut walk_ctx = WalkContext {
            mapper: &mut PageOffsetPgTableMapper {},
            invalidator: &AllTlbInvalidator {},
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
            invalidator: &AllTlbInvalidator, // 修改这里
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
            invalidator: &AllTlbInvalidator, // 修改这里
        };

        walk_and_modify_region(self.l0_table, region, &mut walk_ctx, |va, pgd| {
            if let Some(addr) = pgd.mapped_address() {
                // COW 逻辑：克隆页面，增加引用计数
                let page_region = PhysMemoryRegion::new(addr, PAGE_SIZE);
                let alloc1 = unsafe { PAGE_ALLOC.get().unwrap().alloc_from_region(page_region) };
                
                // 增加引用计数 (Leak 两次是为了模拟引用计数增加，具体取决于你的 FrameAllocator 实现)
                alloc1.clone().leak();
                alloc1.leak();

                let mut ctx = MappingContext {
                    allocator: &mut PageTableAllocator::new(),
                    mapper: &mut PageOffsetPgTableMapper {},
                    invalidator: &AllTlbInvalidator, // 修改这里
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