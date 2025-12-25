use crate::memory::{INITAL_ALLOCATOR, PageOffsetTranslator};

// 确保在 src/arch/riscv64/memory/mod.rs 中导出了 mmu 和 tlb
// 确保在 src/arch/riscv64/memory/mmu/mod.rs 中导出了 smalloc_page_allocator
use super::super::memory::{
    fixmap::{FIXMAPS, Fixmap},
    mmu::smalloc_page_allocator::SmallocPageAlloc,
    //tlb::SfenceTlbInvalidator, 
};

use libkernel::{
    arch::riscv64::memory::{
        pg_descriptors::MemoryType,
        pg_tables::{
            L0Table, MapAttributes, MappingContext, PageTableMapper, PgTable, PgTableArray,
            map_range,
        },
        tlb::AllTlbInvalidator,
    },
    error::Result,
    memory::{
        address::{TPA, TVA},
        permissions::PtePermissions,
    },
};

/// Fixmap 映射器
/// 
/// 在构建页表期间，我们需要修改物理页的内容（写入 PTE）。
/// 由于此时线性映射可能尚未建立，我们利用 Fixmap 机制将物理页临时映射到
/// 虚拟地址空间的一个固定窗口来访问它。
pub struct FixmapMapper<'a> {
    pub fixmaps: &'a mut Fixmap,
}

impl PageTableMapper for FixmapMapper<'_> {
    unsafe fn with_page_table<T: PgTable, R>(
        &mut self,
        pa: TPA<PgTableArray<T>>,
        f: impl FnOnce(TVA<PgTableArray<T>>) -> R,
    ) -> Result<R> {
        // 创建临时映射 guard
        let guard = self.fixmaps.temp_remap_page_table(pa)?;

        // 在闭包中使用映射后的虚拟地址
        Ok(f(unsafe { guard.get_va() }))
    }
}

/// 建立内核的逻辑映射（Direct Mapping / Linear Mapping）
///
/// 将所有物理内存区域映射到内核虚拟地址空间的特定偏移处。
/// 
/// # 参数
/// * `pgtbl_base`: 根页表（L0Table）的物理基地址。
pub fn setup_logical_map(pgtbl_base: TPA<PgTableArray<L0Table>>) -> Result<()> {
    // 1. 获取全局锁
    // 在启动阶段，竞争风险较低，但仍需关中断以防万一
    let mut fixmaps = FIXMAPS.lock_save_irq();
    let mut alloc = INITAL_ALLOCATOR.lock_save_irq();
    let alloc = alloc.as_mut().unwrap();
    
    // 获取物理内存布局（从 Device Tree 或 UEFI 获取）
    let mem_list = alloc.get_memory_list();

    // 2. 准备组件
    let mut mapper = FixmapMapper {
        fixmaps: &mut fixmaps,
    };
    
    // 使用 Boot 阶段的简单分配器
    let mut pg_alloc = SmallocPageAlloc::new(alloc);
    
    // 创建 TLB 失效器
    let invalidator = AllTlbInvalidator {};

    // 3. 构建映射上下文
    let mut ctx = MappingContext {
        allocator: &mut pg_alloc,
        mapper: &mut mapper,
        invalidator: &invalidator,
    };

    // 4. 遍历并映射所有物理内存区域
    for mem_region in mem_list.iter() {
        // 构造映射属性
        let map_attrs = MapAttributes {
            phys: mem_region, // PhysMemoryRegion 通常实现了 Copy
            virt: mem_region.map_via::<PageOffsetTranslator>(),
            mem_type: MemoryType::Normal, // 普通内存（Cacheable）
            perms: PtePermissions::rw(false), // RW 权限，不可执行(X=0)，仅限内核(U=0)
        };

        // 执行映射
        map_range(pgtbl_base, map_attrs, &mut ctx)?;
    }

    Ok(())
}