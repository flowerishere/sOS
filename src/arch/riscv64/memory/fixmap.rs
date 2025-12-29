use super::{FIXMAP_BASE, tlb::SfenceTlbInvalidator as RvTlbInvalidator};
use crate::{
    arch::riscv64::fdt::MAX_FDT_SZ,
    ksym_pa,
    sync::SpinLock,
};
use core::{
    ops::{Deref, DerefMut},
    ptr::NonNull,
};

use fdt_parser::Fdt;

use libkernel::{
    arch::riscv64::memory::{
        pg_descriptors::{
            L0Descriptor, L1Descriptor, L2Descriptor, L3Descriptor,
            MemoryType, PaMapper, PageTableEntry, TableMapper,
        },
        pg_tables::{
            L0Table, L1Table, L2Table, L3Table,
            PgTable, PgTableArray
        },
        tlb::AllTlbInvalidator,
    },
    error::{KernelError, Result},
    memory::{
        PAGE_SIZE,
        address::{IdentityTranslator, PA, TPA, TVA, VA},
        permissions::PtePermissions,
        region::PhysMemoryRegion,
    },
};
use super::IMAGE_BASE;
fn debug_uart_putc(c: u8) {
    unsafe {
        let ptr = 0x1000_0000 as *mut u8; 
        core::ptr::write_volatile(ptr, c);
    }
}

fn debug_print(s: &str) {
    for c in s.bytes() {
        debug_uart_putc(c);
    }
}

fn debug_print_hex(mut val: usize) {
    let hex_chars = b"0123456789abcdef";
    let mut buf = [0u8; 16];
    let mut i = 0;
    
    if val == 0 {
        debug_uart_putc(b'0');
        return;
    }
    
    while val > 0 {
        buf[i] = hex_chars[val & 0xf];
        val >>= 4;
        i += 1;
    }
    
    while i > 0 {
        i -= 1;
        debug_uart_putc(buf[i]);
    }
}
type RvRoot = L0Table;
type RvRootDesc = L0Descriptor;

type RvL1 = L1Table;
type RvL1Desc = L1Descriptor;

type RvL2 = L2Table;
type RvL2Desc = L2Descriptor;

type RvLeaf = L3Table;
type RvLeafDesc = L3Descriptor;

pub struct TempFixmapGuard<T> {
    fixmap: *mut Fixmap,
    va: TVA<T>,
}

impl<T> TempFixmapGuard<T> {
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

    _DtbEnd = MAX_FDT_SZ / PAGE_SIZE, 
    PgTableTmp,
}
pub struct Fixmap {
    l1: PgTableArray<RvL1>,
    l2: PgTableArray<RvL2>,
    l3: [PgTableArray<RvLeaf>; 2],
}

unsafe impl Send for Fixmap {}
unsafe impl Sync for Fixmap {}

pub static FIXMAPS: SpinLock<Fixmap> = SpinLock::new(Fixmap::new());

impl Fixmap {
    pub const fn new() -> Self {
        Self {
            l1: PgTableArray::new(),
            l2: PgTableArray::new(),
            l3: [const { PgTableArray::new() }; 2],
        }
    }

pub fn setup_fixmaps(&mut self, root_base: TPA<PgTableArray<RvRoot>>) {
        debug_print("\n[DEBUG] 1. setup_fixmaps entry\n");
        debug_print("[DEBUG] root_base PA: 0x");
        debug_print_hex(root_base.value()); 
        debug_print("\n");

        if root_base.value() == 0 {
            debug_print("[FATAL] root_base is 0! Caller passed invalid PA.\n");
            loop {}
        }

        let root_va = TVA::from_value(root_base.value());
        let root_table = unsafe { RvRoot::from_ptr(root_va) };
        let invalidator = AllTlbInvalidator {};

        debug_print("[DEBUG] 2. Checking self.l1 alignment...\n");
        
        let l1_ptr = &self.l1 as *const _ as usize;
        debug_print("[DEBUG] self.l1 VA: 0x");
        debug_print_hex(l1_ptr);
        debug_print("\n");

        if l1_ptr & 0xFFF != 0 {
            debug_print("[FATAL] self.l1 is NOT 4KB aligned! Add #[repr(align(4096))] to Fixmap struct.\n");
            loop {}
        }

        debug_print("[DEBUG] 3. Calculating PA safely...\n");
        
        let l1_pa_val = if l1_ptr < 0xFFFF_0000_0000_0000 {
            debug_print("[DEBUG] Address is Low (Identity/Phys), using directly.\n");
            l1_ptr
        } else {
            debug_print("[DEBUG] Address is High, using ksym_pa! macro.\n");
            ksym_pa!(self.l1).value()
        };

        debug_print("[DEBUG] self.l1 PA: 0x");
        debug_print_hex(l1_pa_val); 
        debug_print("\n");

        debug_print("[DEBUG] 4. Creating descriptor...\n");
        let desc = RvRootDesc::new_next_table(PA::from_value(l1_pa_val));

        debug_print("[DEBUG] 5. Writing to Root Table...\n");
        root_table.set_desc(
            FIXMAP_BASE,
            desc,
            &invalidator,
        );
        debug_print("[DEBUG] Root set_desc OK\n");
        
        let l2_ptr = &self.l2 as *const _ as usize;
        let l2_pa_val = if l2_ptr < 0xFFFF_0000_0000_0000 { l2_ptr } else { ksym_pa!(self.l2).value() };
        
        RvL1::from_ptr(TVA::from_ptr(&mut self.l1 as *mut _)).set_desc(
            FIXMAP_BASE,
            RvL1Desc::new_next_table(PA::from_value(l2_pa_val)),
            &invalidator,
        );

        let l3_0_ptr = &self.l3[0] as *const _ as usize;
        let l3_0_pa_val = if l3_0_ptr < 0xFFFF_0000_0000_0000 { l3_0_ptr } else { ksym_pa!(self.l3[0]).value() };

        RvL2::from_ptr(TVA::from_ptr(&mut self.l2 as *mut _)).set_desc(
            FIXMAP_BASE,
            RvL2Desc::new_next_table(PA::from_value(l3_0_pa_val)),
            &invalidator,
        );

        let l3_1_ptr = &self.l3[1] as *const _ as usize;
        let l3_1_pa_val = if l3_1_ptr < 0xFFFF_0000_0000_0000 { l3_1_ptr } else { ksym_pa!(self.l3[1]).value() };
        
        let l2_entry_coverage = 1 << 21; 
        RvL2::from_ptr(TVA::from_ptr(&mut self.l2 as *mut _)).set_desc(
            VA::from_value(FIXMAP_BASE.value() + l2_entry_coverage),
            RvL2Desc::new_next_table(PA::from_value(l3_1_pa_val)),
            &invalidator,
        );
        
        debug_print("[DEBUG] Fixmap setup complete.\n");
    }
    pub unsafe fn remap_fdt(&mut self, fdt_ptr: TPA<u8>) -> Result<VA> {
        let fdt = unsafe { Fdt::from_ptr(NonNull::new_unchecked(fdt_ptr.as_ptr_mut())) }
             .map_err(|_| KernelError::InvalidValue)?;

        let sz = fdt.total_size();
        if sz > MAX_FDT_SZ {
            return Err(KernelError::TooLarge);
        }

        let mut phys_region = PhysMemoryRegion::new(fdt_ptr.to_untyped(), sz);
        let mut va = FIXMAP_BASE;
        let invalidator = AllTlbInvalidator {};

        while phys_region.size() > 0 {
            RvLeaf::from_ptr(TVA::from_ptr_mut(&mut self.l3[0] as *mut _)).set_desc(
                va,
                RvLeafDesc::new_map_pa(
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
        let invalidator = AllTlbInvalidator {};

        RvLeaf::from_ptr(TVA::from_ptr_mut(&mut self.l3[1] as *mut _)).set_desc(
            va,
            RvLeafDesc::new_map_pa(
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
        let invalidator = AllTlbInvalidator {};

        RvLeaf::from_ptr(TVA::from_ptr_mut(&mut self.l3[1] as *mut _)).set_desc(
            va,
            RvLeafDesc::invalid(),
            &invalidator,
        );
    }

    fn va_for_slot(slot: FixmapSlot) -> VA {
        match slot {
            FixmapSlot::DtbStart => FIXMAP_BASE,
            FixmapSlot::_DtbEnd => FIXMAP_BASE,
            FixmapSlot::PgTableTmp => {
                VA::from_value(FIXMAP_BASE.value() + (1 << 21))
            }
        }
    }
}