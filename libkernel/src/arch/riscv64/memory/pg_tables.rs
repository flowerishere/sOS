use core::marker::PhantomData;
use crate::memory::{
    address::{VA, TVA, TPA}, 
    region::{PhysMemoryRegion, VirtMemoryRegion},
    permissions::PtePermissions,
};
use super::pg_descriptors::{Sv39Descriptor, PageTableEntry, MemoryType};
use super::tlb::TLBInvalidator;

pub const DESCRIPTORS_PER_PAGE: usize = 512;
pub const LEVEL_MASK: usize = 0x1FF;

// --- Context Structs for Walker ---
pub struct MapAttributes {
    pub phys: PhysMemoryRegion,
    pub virt: VirtMemoryRegion,
    pub mem_type: MemoryType,
    pub perms: PtePermissions,
}

pub struct MappingContext<'a, A: ?Sized, M: ?Sized, I: ?Sized> {
    pub allocator: &'a mut A,
    pub mapper: &'a mut M,
    pub invalidator: &'a I,
}

pub trait PgTable: Clone + Copy {
    const SHIFT: usize;
    type NextLevel: PgTable;
    
    fn is_leaf() -> bool;
    fn page_size() -> usize { 1 << Self::SHIFT }

    fn index(addr: usize) -> usize {
        (addr >> Self::SHIFT) & LEVEL_MASK
    }
}

#[derive(Clone)]
#[repr(C, align(4096))]
pub struct PgTableArray<K: PgTable> {
    pub entries: [Sv39Descriptor; DESCRIPTORS_PER_PAGE],
    _phantom: PhantomData<K>,
}

macro_rules! impl_pgtable {
    ($table:ident, $shift:expr, $next:ty, $leaf:expr) => {
        #[derive(Clone, Copy)]
        pub struct $table;
        
        impl PgTable for $table {
            const SHIFT: usize = $shift;
            type NextLevel = $next;
            fn is_leaf() -> bool { $leaf }
        }
    };
}

impl_pgtable!(L2Table, 30, L1Table, false); // Root
impl_pgtable!(L1Table, 21, L0Table, false);
impl_pgtable!(L0Table, 12, L0Table, true);  // Leaf

pub type RvPageTableRoot = PgTableArray<L2Table>;

pub trait PageTableMapper {
    unsafe fn with_page_table<T: PgTable, R>(
        &mut self,
        pa: TPA<PgTableArray<T>>,
        f: impl FnOnce(TVA<PgTableArray<T>>) -> crate::error::Result<R>,
    ) -> crate::error::Result<R>;
}

pub trait PageAllocator {
    fn allocate_page_table<T: PgTable>(&mut self) -> crate::error::Result<TPA<PgTableArray<T>>>;
}