use libkernel::{
    arch::riscv64::memory::pg_tables::{PageTableMapper, PgTable, PgTableArray},
    error::Result,
    memory::address::{TPA, TVA},
};
use crate::memory::PageOffsetTranslator;

pub struct PageOffsetPgTableMapper {}

impl PageTableMapper for PageOffsetPgTableMapper {
    unsafe fn with_page_table<T: PgTable, R>(
        &mut self,
        pa: TPA<PgTableArray<T>>,
        f: impl FnOnce(TVA<PgTableArray<T>>) -> R,
    ) -> Result<R> {
        // Assume we have a linear mapping for all physical memory
        Ok(f(pa.to_va::<PageOffsetTranslator>()))
    }
}