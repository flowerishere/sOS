use crate::memory::PageOffsetTranslator;
use libkernel::{
    // 注意这里修改为 riscv64
    arch::riscv64::memory::pg_tables::{PageAllocator, PgTable, PgTableArray},
    error::Result,
    memory::{PAGE_SIZE, address::TPA, smalloc::Smalloc},
};

pub struct SmallocPageAlloc<'a> {
    smalloc: &'a mut Smalloc<PageOffsetTranslator>,
}

impl<'a> SmallocPageAlloc<'a> {
    pub fn new(smalloc: &'a mut Smalloc<PageOffsetTranslator>) -> Self {
        Self { smalloc }
    }
}

impl PageAllocator for SmallocPageAlloc<'_> {
    fn allocate_page_table<T: PgTable>(&mut self) -> Result<TPA<PgTableArray<T>>> {
        // RISC-V 页表节点通常也是 4KiB 大小 (PAGE_SIZE)
        // 使用 smalloc 分配物理内存
        Ok(TPA::from_value(
            self.smalloc.alloc(PAGE_SIZE, PAGE_SIZE)?.value(),
        ))
    }
}