use core::marker::PhantomData;

use super::{
    pg_descriptors::{
        L0Descriptor, L1Descriptor, L2Descriptor, L3Descriptor, MemoryType, PaMapper,
        PageTableEntry, TableMapper,
    },
    tlb::TLBInvalidator,
};
use crate::{
    error::{MapError, Result},
    memory::{
        PAGE_SIZE,
        address::{TPA, TVA, VA},
        permissions::PtePermissions,
        region::{PhysMemoryRegion, VirtMemoryRegion},
    },
};

pub const DESCRIPTORS_PER_PAGE: usize = PAGE_SIZE / core::mem::size_of::<u64>();
pub const LEVEL_MASK: usize = DESCRIPTORS_PER_PAGE - 1;

/// Trait representing a single level of the page table hierarchy.
pub trait PgTable: Clone + Copy {
    /// Bit shift used to extract the index for this page table level.
    const SHIFT: usize;

    /// The descriptor (page table entry) type for this level.
    type Descriptor: PageTableEntry;

    fn from_ptr(ptr: TVA<PgTableArray<Self>>) -> Self;

    fn to_raw_ptr(self) -> *mut u64;

    /// Compute the index into this page table from a virtual address.
    fn pg_index(va: VA) -> usize {
        (va.value() >> Self::SHIFT) & LEVEL_MASK
    }

    /// Get the descriptor for a given virtual address.
    fn get_desc(self, va: VA) -> Self::Descriptor;

    /// Set the value of the descriptor for a particular VA.
    fn set_desc(self, va: VA, desc: Self::Descriptor, invalidator: &dyn TLBInvalidator);
}

pub(super) trait TableMapperTable: PgTable<Descriptor: TableMapper> + Clone + Copy {
    type NextLevel: PgTable;

    #[allow(dead_code)]
    fn next_table_pa(self, va: VA) -> Option<TPA<PgTableArray<Self::NextLevel>>> {
        let desc = self.get_desc(va);
        Some(TPA::from_value(desc.next_table_address()?.value()))
    }
}

#[derive(Clone)]
#[repr(C, align(4096))]
pub struct PgTableArray<K: PgTable> {
    pages: [u64; DESCRIPTORS_PER_PAGE],
    _phantom: PhantomData<K>,
}

impl<K: PgTable> PgTableArray<K> {
    pub const fn new() -> Self {
        Self {
            pages: [0; DESCRIPTORS_PER_PAGE],
            _phantom: PhantomData,
        }
    }
}

impl<K: PgTable> Default for PgTableArray<K> {
    fn default() -> Self {
        Self::new()
    }
}

macro_rules! impl_pgtable {
    ($table:ident, $shift:expr, $desc_type:ident) => {
        #[derive(Clone, Copy)]
        pub struct $table {
            base: *mut u64,
        }

        impl PgTable for $table {
            const SHIFT: usize = $shift;
            type Descriptor = $desc_type;

            fn from_ptr(ptr: TVA<PgTableArray<Self>>) -> Self {
                Self {
                    base: ptr.as_ptr_mut().cast(),
                }
            }

            fn to_raw_ptr(self) -> *mut u64 {
                self.base
            }

            fn get_desc(self, va: VA) -> Self::Descriptor {
                let raw = unsafe { self.base.add(Self::pg_index(va)).read_volatile() };
                Self::Descriptor::from_raw(raw)
            }

            fn set_desc(self, va: VA, desc: Self::Descriptor, _invalidator: &dyn TLBInvalidator) {
                unsafe {
                    self.base
                        .add(Self::pg_index(va))
                        .write_volatile(PageTableEntry::as_raw(desc))
                };
                // In RISC-V, we typically need to run `sfence.vma` when modifying PTEs 
                // that were valid. The `invalidator` trait usually abstracts this.
            }
        }
    };
}

// RISC-V Sv48 Shifts:
// Level 0 (Root): 39
// Level 1: 30
// Level 2: 21
// Level 3 (Leaf): 12

impl_pgtable!(L0Table, 39, L0Descriptor);
impl TableMapperTable for L0Table {
    type NextLevel = L1Table;
}

impl_pgtable!(L1Table, 30, L1Descriptor);
impl TableMapperTable for L1Table {
    type NextLevel = L2Table;
}

impl_pgtable!(L2Table, 21, L2Descriptor);
impl TableMapperTable for L2Table {
    type NextLevel = L3Table;
}

impl_pgtable!(L3Table, 12, L3Descriptor);

pub trait PageTableMapper {
    unsafe fn with_page_table<T: PgTable, R>(
        &mut self,
        pa: TPA<PgTableArray<T>>,
        f: impl FnOnce(TVA<PgTableArray<T>>) -> R,
    ) -> Result<R>;
}

pub trait PageAllocator {
    fn allocate_page_table<T: PgTable>(&mut self) -> Result<TPA<PgTableArray<T>>>;
}

pub struct MapAttributes {
    pub phys: PhysMemoryRegion,
    pub virt: VirtMemoryRegion,
    pub mem_type: MemoryType,
    pub perms: PtePermissions,
}

pub struct MappingContext<'a, PA, PM>
where
    PA: PageAllocator + 'a,
    PM: PageTableMapper + 'a,
{
    pub allocator: &'a mut PA,
    pub mapper: &'a mut PM,
    pub invalidator: &'a dyn TLBInvalidator,
}

pub fn map_range<PA, PM>(
    l0_table: TPA<PgTableArray<L0Table>>,
    mut attrs: MapAttributes,
    ctx: &mut MappingContext<PA, PM>,
) -> Result<()>
where
    PA: PageAllocator,
    PM: PageTableMapper,
{
    if attrs.phys.size() != attrs.virt.size() {
        Err(MapError::SizeMismatch)?
    }

    if attrs.phys.size() < PAGE_SIZE {
        Err(MapError::TooSmall)?
    }

    if !attrs.phys.is_page_aligned() {
        Err(MapError::PhysNotAligned)?
    }

    if !attrs.virt.is_page_aligned() {
        Err(MapError::VirtNotAligned)?
    }

    while attrs.virt.size() > 0 {
        let va = attrs.virt.start_address();

        // Try mapping at L1 (1GB blocks)
        let l1 = map_at_level(l0_table, va, ctx)?;
        if let Some(pgs_mapped) = try_map_pa(l1, va, attrs.phys, &attrs, ctx)? {
            attrs.virt = attrs.virt.add_pages(pgs_mapped);
            attrs.phys = attrs.phys.add_pages(pgs_mapped);
            continue;
        }

        // Try mapping at L2 (2MB blocks)
        let l2 = map_at_level(l1, va, ctx)?;
        if let Some(pgs_mapped) = try_map_pa(l2, va, attrs.phys, &attrs, ctx)? {
            attrs.virt = attrs.virt.add_pages(pgs_mapped);
            attrs.phys = attrs.phys.add_pages(pgs_mapped);
            continue;
        }

        // Map at L3 (4KB pages)
        let l3 = map_at_level(l2, va, ctx)?;
        try_map_pa(l3, va, attrs.phys, &attrs, ctx)?;

        attrs.virt = attrs.virt.add_pages(1);
        attrs.phys = attrs.phys.add_pages(1);
    }

    Ok(())
}

fn try_map_pa<L, PA, PM>(
    table: TPA<PgTableArray<L>>,
    va: VA,
    phys_region: PhysMemoryRegion,
    attrs: &MapAttributes,
    ctx: &mut MappingContext<PA, PM>,
) -> Result<Option<usize>>
where
    L: PgTable<Descriptor: PaMapper>,
    PA: PageAllocator,
    PM: PageTableMapper,
{
    if L::Descriptor::could_map(phys_region, va) {
        unsafe {
            if ctx
                .mapper
                .with_page_table(table, |tbl| L::from_ptr(tbl).get_desc(va))?
                .is_valid()
            {
                return Err(MapError::AlreadyMapped)?;
            }

            ctx.mapper.with_page_table(table, |tbl| {
                L::from_ptr(tbl).set_desc(
                    va,
                    L::Descriptor::new_map_pa(
                        phys_region.start_address(),
                        attrs.mem_type,
                        attrs.perms,
                    ),
                    ctx.invalidator,
                );
            })?;
        }

        Ok(Some(1 << (L::Descriptor::map_shift() - 12)))
    } else {
        Ok(None)
    }
}

pub(super) fn map_at_level<L, PA, PM>(
    table: TPA<PgTableArray<L>>,
    va: VA,
    ctx: &mut MappingContext<PA, PM>,
) -> Result<TPA<PgTableArray<L::NextLevel>>>
where
    L: TableMapperTable,
    PA: PageAllocator,
    PM: PageTableMapper,
{
    unsafe {
        let desc = ctx
            .mapper
            .with_page_table(table, |pgtable| L::from_ptr(pgtable).get_desc(va))?;

        if let Some(pa) = desc.next_table_address() {
            return Ok(TPA::from_value(pa.value()));
        }

        if desc.is_valid() {
            return Err(MapError::AlreadyMapped)?;
        }

        let new_pa = ctx.allocator.allocate_page_table::<L::NextLevel>()?;

        ctx.mapper.with_page_table(new_pa, |new_pgtable| {
            core::ptr::write_bytes(new_pgtable.as_ptr_mut() as *mut _ as *mut u8, 0, PAGE_SIZE)
        })?;

        ctx.mapper.with_page_table(table, |pgtable| {
            L::from_ptr(pgtable).set_desc(
                va,
                L::Descriptor::new_next_table(new_pa.to_untyped()),
                ctx.invalidator,
            );
        })?;

        Ok(new_pa)
    }
}
pub type RvPageTableRoot = L0Table;