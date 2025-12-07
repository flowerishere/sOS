use core::marker::PhantomData;
use core::ops::{Deref, DerefMut};

use libkernel::{
    error::Result,
    memory::{
        PAGE_SIZE,
        address::{PA, TPA, TVA},
        permissions::Permissions,
        region::{PhysMemRegion, VirtMemRegion},
    },
};

use super::{
    pg_descriptors::{MemoryType, PageDescriptor},
    pg_walk::Walk,
    tlb::TlbInvalidate,
};

/// a trait for allocating page tables
pub trait PageTableAllocator {
    fn allocate_page_table<T: PgTable>(&mut self) -> Result<T>;
}

/// a trait for mapping functionality(translating PA to VA during mapping)
pub trait PageTableMapper {
    unsafe fn with_page_table<T: PgTable, R>(
        &mut self,
        pa: TPA<PgTableArray<T>>,
        f: impl FnOnce(TVA<PgTableArray<T>>) -> R,
    ) -> Result<R>;
}

/// context passed around during page table operations
pub struct MappingContext<'a> {
    pub allocator: &'a mut dyn PageAllocator,
    pub mapper: &'a mut dyn PageTableMapper,
    pub invalidator: &'a dyn TlbInvalidator,
}

/// attributes for a memory mapping
#[derive(Clone, Copy, Debug)]
pub struct MapAttributes {
    pub phys: PhysMemRegion,
    pub virt: VirtMemRegion,
    pub mem_type: MemoryType,
    pub perms: Permissions,
}

/// representation of an array of 512 page descriptors(4KiB)
#[repr(C, align(4096))]
pub struct PgTableArray<T: PgTable> {
    pub entries: [PageDescriptor; 512],
    phantom: PhantomData<T>,
}

impl<T: PgTable> Deref for PgTableArray<T> {
    fn deref_mut(&mut self) -> &mut Self::Target {
        &mut self.entries
    }
}

///trait marking a specific level in the page table hierarchy
pub trait PgTable :Sized {
    type NextLevel: PgTable;

    ///returns the index of this level for a given virtual address
    /// Sv48 levels:3(Root), 2, 1, 0(Leaf)
    fn index(va: usize) -> usize;

    ///returns the page size sovered by an entry at this level
    fn page_size() -> usize;

    ///return true if this level is the last level(leaf)
    fn is_leaf() -> bool;
}

/// ---Level Definitions for Sv48 ---

pub enum L0Table {}
pub enum L1Table {}
pub enum L2Table {}
pub enum L3Table {}

// only L3 is truly a leaf in standard 4K paging
// riscv supports superpages at L1(2MB) and L2(1GB)
//for simplicity in this implementation, we designate L3 as the leaf target

impl PgTable for L0Table {
    type NextLevel = L1Table;
    fn index(va: usize) -> usize { (va >> 39) & 0x1FF }
    fn page_size() -> usize { 512 * 1024 * 1024 * 1024 } //512GB
    fn is_leaf() -> bool { false }
}

impl PgTable for L1Table {
    type NextLevel = L2Table;
    fn index(va: usize) -> usize { (va >> 30) & 0x1FF }
    fn page_size() -> usize { 1 * 1024 * 1024 * 1024 } //1GB
    fn is_leaf() -> bool { false }
}

impl PgTable for L2Table {
    type NextLevel = L3Table;
    fn index(va: usize) -> usize { (va >> 21) & 0x1FF }
    fn page_size() -> usize { 2 * 1024 * 1024 } //2MB
    fn is_leaf() -> bool { false }
}

impl PgTable for L3Table {
    type NextLevel = L3Table; //no next level
    fn index(va: usize) -> usize { (va >> 12) & 0x1FF }
    fn page_size() -> usize { 4 * 1024 } //4KB
    fn is_leaf() -> bool { true }
}

///entry point to map a range of memory
pub fn map_range(
    root:TPA<PgTableArray<L0Table>>,
    attrs: MapAttributes,
    ctx: &mut MappingContext,
) -> Result<()> {
    //Delete to the walker implementation
    Walk::<L0Table>::map_range(root, attrs, ctx)
}