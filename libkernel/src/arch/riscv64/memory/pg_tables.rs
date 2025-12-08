use core::marker::PhantomData;
use core::ops::{Deref, DerefMut};

use crate::{
    error::Result,
    memory::{
        PAGE_SIZE,
        address::{TPA, TVA},
        permissions::PtePermissions,
        region::{PhysMemoryRegion, VirtMemoryRegion},
    },
};

use super::{
    pg_descriptors::{MemoryType, PageDescriptor},
    pg_walk::Walk,
    tlb::TlbInvalidator,
};

/// A trait for allocating page tables.
pub trait PageAllocator {
    fn allocate_page_table<T: PgTable>(&mut self) -> Result<TPA<PgTableArray<T>>>;
}

/// A trait for mapping functionality (translating PA to VA during table walks).
pub trait PageTableMapper {
    unsafe fn with_page_table<T: PgTable, R>(
        &mut self,
        pa: TPA<PgTableArray<T>>,
        f: impl FnOnce(TVA<PgTableArray<T>>) -> R,
    ) -> Result<R>;
}

/// Context passed around during page table operations.
/// 
/// Uses generics (A, M, I) instead of trait objects to avoid "not dyn compatible" 
/// errors, as PageAllocator and PageTableMapper have generic methods.
pub struct MappingContext<'a, A, M, I>
where
    A: PageAllocator + ?Sized,
    M: PageTableMapper + ?Sized,
    I: TlbInvalidator + ?Sized,
{
    pub allocator: &'a mut A,
    pub mapper: &'a mut M,
    pub invalidator: &'a I,
}

/// Attributes for a memory mapping.
#[derive(Debug, Clone, Copy)]
pub struct MapAttributes {
    pub phys: PhysMemoryRegion,
    pub virt: VirtMemoryRegion,
    pub mem_type: MemoryType,
    pub perms: PtePermissions,
}

/// Representation of an array of 512 page descriptors (4KiB).
#[repr(C, align(4096))]
pub struct PgTableArray<T: PgTable> {
    pub entries: [PageDescriptor; 512],
    phantom: PhantomData<T>,
}

impl<T: PgTable> Deref for PgTableArray<T> {
    type Target = [PageDescriptor; 512];
    fn deref(&self) -> &Self::Target {
        &self.entries
    }
}

impl<T: PgTable> DerefMut for PgTableArray<T> {
    fn deref_mut(&mut self) -> &mut Self::Target {
        &mut self.entries
    }
}

/// Trait marking a specific level in the page table hierarchy.
pub trait PgTable: Sized {
    type NextLevel: PgTable;
    
    /// Returns the index of this level for a given virtual address.
    fn index(va: usize) -> usize;
    
    /// Returns the page size covered by an entry at this level.
    fn page_size() -> usize;
    
    /// Returns true if this level is the last level (Leaf).
    fn is_leaf() -> bool;
}

// --- Level Definitions for Sv48 (4 Levels) ---
// L0 = Root (Level 3 in HW) -> 512GB range
// L1 = Level 2 in HW -> 1GB range
// L2 = Level 1 in HW -> 2MB range
// L3 = Level 0 in HW -> 4KB range (Leaf)

pub enum L0Table {}
pub enum L1Table {}
pub enum L2Table {}
pub enum L3Table {}

impl PgTable for L0Table {
    type NextLevel = L1Table;
    fn index(va: usize) -> usize { (va >> 39) & 0x1FF }
    fn page_size() -> usize { 512 * 1024 * 1024 * 1024 } // 512 GB
    fn is_leaf() -> bool { false }
}

impl PgTable for L1Table {
    type NextLevel = L2Table;
    fn index(va: usize) -> usize { (va >> 30) & 0x1FF }
    fn page_size() -> usize { 1 * 1024 * 1024 * 1024 } // 1 GB
    fn is_leaf() -> bool { false }
}

impl PgTable for L2Table {
    type NextLevel = L3Table;
    fn index(va: usize) -> usize { (va >> 21) & 0x1FF }
    fn page_size() -> usize { 2 * 1024 * 1024 } // 2 MB
    fn is_leaf() -> bool { false }
}

impl PgTable for L3Table {
    type NextLevel = L3Table; // Recursive definition, unused for leaf
    fn index(va: usize) -> usize { (va >> 12) & 0x1FF }
    fn page_size() -> usize { PAGE_SIZE } // 4 KB
    fn is_leaf() -> bool { true }
}

/// Entry point to map a range of memory.
pub fn map_range<A, M, I>(
    root: TPA<PgTableArray<L0Table>>,
    attrs: MapAttributes,
    ctx: &mut MappingContext<A, M, I>,
) -> Result<()>
where
    A: PageAllocator + ?Sized,
    M: PageTableMapper + ?Sized,
    I: TlbInvalidator + ?Sized,
{
    Walk::<L0Table>::map_range(root, attrs, ctx)
}