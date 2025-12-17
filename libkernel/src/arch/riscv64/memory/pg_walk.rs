use super::{
    pg_descriptors::{L3Descriptor, PageTableEntry, TableMapper},
    pg_tables::{L0Table, L3Table, PageTableMapper, PgTable, PgTableArray, TableMapperTable},
    tlb::{NullTlbInvalidator, TLBInvalidator},
};
use crate::{
    error::{MapError, Result},
    memory::{
        PAGE_SIZE,
        address::{TPA, VA},
        region::VirtMemoryRegion,
    },
};

pub struct WalkContext<'a, PM>
where
    PM: PageTableMapper + 'a,
{
    pub mapper: &'a mut PM,
    pub invalidator: &'a dyn TLBInvalidator,
}

trait RecursiveWalker: PgTable + Sized {
    fn walk<F, PM>(
        table_pa: TPA<PgTableArray<Self>>,
        region: VirtMemoryRegion,
        ctx: &mut WalkContext<PM>,
        modifier: &mut F,
    ) -> Result<()>
    where
        PM: PageTableMapper,
        F: FnMut(VA, L3Descriptor) -> L3Descriptor;
}

impl<T> RecursiveWalker for T
where
    T: TableMapperTable,
    T::NextLevel: RecursiveWalker,
{
    fn walk<F, PM>(
        table_pa: TPA<PgTableArray<Self>>,
        region: VirtMemoryRegion,
        ctx: &mut WalkContext<PM>,
        modifier: &mut F,
    ) -> Result<()>
    where
        PM: PageTableMapper,
        F: FnMut(VA, L3Descriptor) -> L3Descriptor,
    {
        let table_coverage = 1 << T::SHIFT;

        let start_idx = Self::pg_index(region.start_address());
        let end_idx = Self::pg_index(region.end_address_inclusive());
        let table_base_va = region.start_address().align(1 << (T::SHIFT + 9));

        for idx in start_idx..=end_idx {
            let entry_va = table_base_va.add_bytes(idx * table_coverage);

            let desc = unsafe {
                ctx.mapper
                    .with_page_table(table_pa, |pgtable| T::from_ptr(pgtable).get_desc(entry_va))?
            };

            if let Some(next_desc) = desc.next_table_address() {
                let sub_region = VirtMemoryRegion::new(entry_va, table_coverage)
                    .intersection(region)
                    .expect("Sub region should overlap with parent region");

                T::NextLevel::walk(next_desc.cast(), sub_region, ctx, modifier)?;
            } else if desc.is_valid() {
                Err(MapError::NotL3Mapped)?
            } else {
                continue;
            }
        }

        Ok(())
    }
}

impl RecursiveWalker for L3Table {
    fn walk<F, PM>(
        table_pa: TPA<PgTableArray<Self>>,
        region: VirtMemoryRegion,
        ctx: &mut WalkContext<PM>,
        modifier: &mut F,
    ) -> Result<()>
    where
        PM: PageTableMapper,
        F: FnMut(VA, L3Descriptor) -> L3Descriptor,
    {
        unsafe {
            ctx.mapper.with_page_table(table_pa, |pgtable| {
                let table = L3Table::from_ptr(pgtable);
                for va in region.iter_pages() {
                    let desc = table.get_desc(va);
                    if desc.is_valid() {
                        table.set_desc(va, modifier(va, desc), ctx.invalidator);
                    }
                }
            })
        }
    }
}

pub fn walk_and_modify_region<F, PM>(
    l0_table: TPA<PgTableArray<L0Table>>,
    region: VirtMemoryRegion,
    ctx: &mut WalkContext<PM>,
    mut modifier: F,
) -> Result<()>
where
    PM: PageTableMapper,
    F: FnMut(VA, L3Descriptor) -> L3Descriptor,
{
    if !region.is_page_aligned() {
        Err(MapError::VirtNotAligned)?;
    }

    if region.size() == 0 {
        return Ok(());
    }

    L0Table::walk(l0_table, region, ctx, &mut modifier)
}

pub fn get_pte<PM: PageTableMapper>(
    l0_table: TPA<PgTableArray<L0Table>>,
    va: VA,
    mapper: &mut PM,
) -> Result<Option<L3Descriptor>> {
    let mut descriptor = None;

    let mut walk_ctx = WalkContext {
        mapper,
        invalidator: &NullTlbInvalidator {},
    };

    walk_and_modify_region(
        l0_table,
        VirtMemoryRegion::new(va.page_aligned(), PAGE_SIZE),
        &mut walk_ctx,
        |_, pte| {
            descriptor = Some(pte);
            pte
        },
    )?;

    Ok(descriptor)
}