// 使用 super::tlb 中刚刚定义的 SfenceTlbInvalidator
use super::{FIXMAP_BASE, tlb::SfenceTlbInvalidator as RvTlbInvalidator};
use crate::{
    arch::riscv64::fdt::MAX_FDT_SZ,
    ksym_pa,
    // 修正点 1: 使用内核 crate 本地的 SpinLock，而不是 libkernel 的
    sync::SpinLock,
};
use core::{
    ops::{Deref, DerefMut},
    ptr::NonNull,
};
// 修正点 2: 确保使用正确的 FDT 解析库
use fdt_parser::Fdt;

use libkernel::{
    arch::riscv64::memory::{
        // 修正点 3: 导入真实存在的描述符类型，而非 RvDescriptor
        pg_descriptors::{
            L0Descriptor, L1Descriptor, L2Descriptor, L3Descriptor,
            MemoryType, PaMapper, PageTableEntry, TableMapper,
        },
        // 修正点 4: 导入真实存在的页表类型
        pg_tables::{
            L0Table, L1Table, L2Table, L3Table,
            PgTable, PgTableArray
        },
        tlb::AllTlbInvalidator,
    },
    error::{KernelError, Result},
    memory::{
        PAGE_SIZE,
        address::{IdentityTranslator, TPA, TVA, VA},
        permissions::PtePermissions,
        region::PhysMemoryRegion,
    },
};

// ============================================================================
// RISC-V SV48 适配层 (4级页表)
// 建立本地别名以匹配 fixmap 的逻辑需求
// ============================================================================

// SV48 根页表是 L0 (Level 0)，覆盖 256TB
type RvRoot = L0Table;
type RvRootDesc = L0Descriptor;

// 中间层级
type RvL1 = L1Table;
type RvL1Desc = L1Descriptor;

type RvL2 = L2Table;
type RvL2Desc = L2Descriptor;

// 叶子层级 (L3)，覆盖 4KB
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
    // FDT 最大尺寸 / 页面大小 = 需要的页数
    // 一个 L3 条目覆盖 4KB，所以这里计算的是 L3 表中的索引偏移
    _DtbEnd = MAX_FDT_SZ / PAGE_SIZE, 
    PgTableTmp,
}

/// RISC-V SV48 Fixmap 存储
/// 
/// 结构：
/// Kernel Root (L0) -> Internal L1 -> Internal L2 -> Internal L3s
///
/// 我们预分配了用于映射 FIXMAP 区域所需的各级页表页。
pub struct Fixmap {
    l1: PgTableArray<RvL1>,
    l2: PgTableArray<RvL2>,
    // l3[0]: 用于 FDT 映射 (覆盖 2MB)
    // l3[1]: 用于临时页表映射 (覆盖 2MB)
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

    /// 初始化 Fixmap 页表结构
    /// 将内部表挂载到内核根页表 (root_base) 上
    pub fn setup_fixmaps(&mut self, root_base: TPA<PgTableArray<RvRoot>>) {
        let root_table = RvRoot::from_ptr(root_base.to_va::<IdentityTranslator>());
        let invalidator = AllTlbInvalidator {};

        // 1. 挂载 L1 到 Root (L0) 的 FIXMAP_BASE 位置
        root_table.set_desc(
            FIXMAP_BASE,
            RvRootDesc::new_next_table(ksym_pa!(self.l1)),
            &invalidator,
        );

        // 2. 挂载 L2 到 L1
        RvL1::from_ptr(TVA::from_ptr(&mut self.l1 as *mut _)).set_desc(
            FIXMAP_BASE,
            RvL1Desc::new_next_table(ksym_pa!(self.l2)),
            &invalidator,
        );

        // 3. 挂载 L3 到 L2
        // Slot 0: FDT (偏移 0)
        RvL2::from_ptr(TVA::from_ptr(&mut self.l2 as *mut _)).set_desc(
            FIXMAP_BASE,
            RvL2Desc::new_next_table(ksym_pa!(self.l3[0])),
            &invalidator,
        );

        // Slot 1: Temp (偏移 2MB)
        let l2_entry_coverage = 1 << 21; // 2MB
        RvL2::from_ptr(TVA::from_ptr(&mut self.l2 as *mut _)).set_desc(
            VA::from_value(FIXMAP_BASE.value() + l2_entry_coverage),
            RvL2Desc::new_next_table(ksym_pa!(self.l3[1])),
            &invalidator,
        );
    }

    /// 映射 FDT 到 Fixmap 区域
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

        // 使用 l3[0] 进行映射
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

    /// 临时映射一个物理页（通常用于修改页表）
    pub fn temp_remap_page_table<T: PgTable>(
        &mut self,
        pa: TPA<PgTableArray<T>>,
    ) -> Result<TempFixmapGuard<PgTableArray<T>>> {
        let va = Self::va_for_slot(FixmapSlot::PgTableTmp);
        let invalidator = AllTlbInvalidator {};

        // 使用 l3[1] 进行映射
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
                // Temp 区域位于 FIXMAP_BASE + 2MB
                VA::from_value(FIXMAP_BASE.value() + (1 << 21))
            }
        }
    }
}