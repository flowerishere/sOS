use super::{FIXMAP_BASE, tlb::RvTlbInvalidator};
use crate::{
    arch::riscv64::fdt::MAX_FDT_SZ,
    ksym_pa, 
    sync::spinlock::SpinLock,
};
use core::{
    ops::{Deref, DerefMut},
    ptr::NonNull,
};
use libkernel::{
    arch::riscv64::memory::{
        pg_descriptors::{
            RvDescriptor, MemoryType, PaMapper, PageTableEntry, TableMapper,
        },
        pg_tables::{
            RvPageTableRoot, RvPageTableL1, RvPageTableL0, PgTable, PgTableArray
        },
    },
    error::{KernelError, Result},
    memory::{
        PAGE_SIZE,
        address::{IdentityTranslator, TPA, TVA, VA},
        permissions::PtePermissions,
        region::PhysMemoryRegion,
    },
};

pub struct TempFixmapGuard<T> {
    fixmap: *mut Fixmap,
    va: TVA<T>,
}

impl<T> TempFixmapGuard<T> {
    /// Get the VA associated with this temp fixmap.
    ///
    /// SAFETY: The returned VA is not tied back to the lifetime of the guard.
    /// Therefore, care *must* be taken that it is not used after the guard has
    /// gone out of scope.
    pub unsafe fn get_va(&self) -> TVA<T> {
        self.va
    }
}

impl<T> Deref for TempFixmapGuard<T> {
    type Target = T;

    fn deref(&self) -> &Self::Target {
        unsafe { self.va.as_ptr().cast::<T>().as_ref().unwrap() }
    }
}

impl<T> DerefMut for TempFixmapGuard<T> {
    fn deref_mut(&mut self) -> &mut Self::Target {
        unsafe { self.va.as_ptr_mut().cast::<T>().as_mut().unwrap() }
    }
}

impl<T> Drop for TempFixmapGuard<T> {
    fn drop(&mut self) {
        unsafe {
            let fixmap = &mut *self.fixmap;
            fixmap.unmap_temp_page();
        }
    }
}

#[derive(Clone, Copy)]
#[repr(usize)]
enum FixmapSlot {
    DtbStart = 0,
    // RISC-V Leaf page table (L0) typically covers 2MiB (512 * 4KB).
    // If MAX_FDT_SZ <= 2MiB, we fit in one L0 table.
    // DtbEnd is relative to the number of pages.
    _DtbEnd = MAX_FDT_SZ / PAGE_SIZE, 
    PgTableTmp,
}

/// Fixmap storage for RISC-V SV39.
/// 
/// SV39 Hierarchy:
/// Root (L2) -> points to L1 (1GB regions)
/// L1        -> points to L0 (2MB regions)
/// L0        -> points to Physical Pages (4KB)
///
/// We provide backing storage for:
/// 1. One L1 table (to hook into the Kernel Root L2).
/// 2. Two L0 tables (one for FDT mapping, one for Temp mapping).
///    Note: Assuming FDT fits in 2MB and Temp fits in 2MB, and they are adjacent in VA space.
pub struct Fixmap {
    l1: PgTableArray<RvPageTableL1>,
    l0: [PgTableArray<RvPageTableL0>; 2],
}

unsafe impl Send for Fixmap {}
unsafe impl Sync for Fixmap {}

pub static FIXMAPS: SpinLock<Fixmap> = SpinLock::new(Fixmap::new());

impl Fixmap {
    pub const fn new() -> Self {
        Self {
            l1: PgTableArray::new(),
            l0: [const { PgTableArray::new() }; 2],
        }
    }

    /// Setup the fixmap page tables.
    /// 
    /// Hooks the internal L1 table into the Kernel's Root (L2) table,
    /// and hooks the internal L0 tables into the internal L1 table.
    pub fn setup_fixmaps(&mut self, root_base: TPA<PgTableArray<RvPageTableRoot>>) {
        let root_table = RvPageTableRoot::from_ptr(root_base.to_va::<IdentityTranslator>());
        let invalidator = RvTlbInvalidator::new();

        // 1. Hook L1 table into Root (L2) at FIXMAP_BASE
        // RISC-V SV39: L2 entry covers 1GB.
        root_table.set_desc(
            FIXMAP_BASE,
            RvDescriptor::new_next_table(ksym_pa!(self.l1)),
            &invalidator,
        );

        // 2. Hook L0 tables into L1 table
        // L0[0] covers the FDT range
        RvPageTableL1::from_ptr(TVA::from_ptr(&mut self.l1 as *mut _)).set_desc(
            FIXMAP_BASE,
            RvDescriptor::new_next_table(ksym_pa!(self.l0[0])),
            &invalidator,
        );

        // L0[1] covers the Temp page range
        // We offset the VA by the size covered by one L0 table (2MB in SV39)
        // 1 << RvPageTableL0::SHIFT (where SHIFT=12, ENTRIES=512) -> 2MB is wrong calculation logic
        // typically defined in the trait.
        // For SV39: 
        // L0 shifts 12 bits (page size).
        // L1 shifts 21 bits (2MB).
        // So the next slot in L1 is at address + (1 << 21).
        let l0_coverage = 1 << 21; // 2MB
        
        RvPageTableL1::from_ptr(TVA::from_ptr(&mut self.l1 as *mut _)).set_desc(
            VA::from_value(FIXMAP_BASE.value() + l0_coverage),
            RvDescriptor::new_next_table(ksym_pa!(self.l0[1])),
            &invalidator,
        );
    }

    /// Remap the FDT via the fixmaps.
    pub unsafe fn remap_fdt(&mut self, fdt_ptr: TPA<u8>) -> Result<VA> {
        // Safe validation of FDT header
        // Note: fdt_parser crate dependency assumed similar to ARM64
        let fdt = unsafe { fdt::Fdt::from_ptr(NonNull::new_unchecked(fdt_ptr.as_ptr_mut())) }
             .map_err(|_| KernelError::InvalidValue)?;

        let sz = fdt.total_size();

        if sz > MAX_FDT_SZ {
            return Err(KernelError::TooLarge);
        }

        let mut phys_region = PhysMemoryRegion::new(fdt_ptr.to_untyped(), sz);
        let mut va = FIXMAP_BASE;
        let invalidator = RvTlbInvalidator::new();

        // Use l0[0] for FDT
        while phys_region.size() > 0 {
            RvPageTableL0::from_ptr(TVA::from_ptr_mut(&mut self.l0[0] as *mut _)).set_desc(
                va,
                RvDescriptor::new_map_pa(
                    phys_region.start_address(),
                    MemoryType::Normal,
                    PtePermissions::ro(false),
                ),
                &invalidator,
            );

            phys_region = phys_region.add_pages(1);
            va = va.add_pages(1);
        }

        Ok(Self::va_for_slot(FixmapSlot::DtbStart))
    }

    pub fn temp_remap_page_table<T: PgTable>(
        &mut self,
        pa: TPA<PgTableArray<T>>,
    ) -> Result<TempFixmapGuard<PgTableArray<T>>> {
        let va = Self::va_for_slot(FixmapSlot::PgTableTmp);
        let invalidator = RvTlbInvalidator::new();

        // Use l0[1] for Temp mappings
        RvPageTableL0::from_ptr(TVA::from_ptr_mut(&mut self.l0[1] as *mut _)).set_desc(
            va,
            RvDescriptor::new_map_pa(
                pa.to_untyped(),
                MemoryType::Normal,
                PtePermissions::rw(false),
            ),
            &invalidator,
        );

        Ok(TempFixmapGuard {
            fixmap: self as *mut _,
            va: va.cast(),
        })
    }

    fn unmap_temp_page(&mut self) {
        let va = Self::va_for_slot(FixmapSlot::PgTableTmp);
        let invalidator = RvTlbInvalidator::new();

        RvPageTableL0::from_ptr(TVA::from_ptr_mut(&mut self.l0[1] as *mut _)).set_desc(
            va,
            RvDescriptor::invalid(),
            &invalidator,
        );
    }

    fn va_for_slot(slot: FixmapSlot) -> VA {
        // Calculate VA based on slot index.
        // Assuming slots are contiguous 4KB pages starting at FIXMAP_BASE.
        // Note: Logic in setup_fixmaps places L0[1] (Temp) at FIXMAP_BASE + 2MB.
        // We must ensure this calculation aligns with where we mapped the L0 tables.
        
        match slot {
            FixmapSlot::DtbStart => FIXMAP_BASE,
            FixmapSlot::_DtbEnd => FIXMAP_BASE, // Not really used for calculation
            FixmapSlot::PgTableTmp => {
                // If we mapped L0[1] at offset 2MB (covering the next 2MB range),
                // and PgTableTmp is the first entry in that L0 table:
                VA::from_value(FIXMAP_BASE.value() + (1 << 21))
            }
        }
    }
}