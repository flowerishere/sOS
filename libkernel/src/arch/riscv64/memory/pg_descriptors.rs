use tock_registers::interfaces::{ReadWriteable, Readable};
use tock_registers::{register_bitfields, registers::InMemoryRegister};

use crate::memory::PAGE_SHIFT;
use crate::memory::address::{PA, VA};
use crate::memory::permissions::PtePermissions;
use crate::memory::region::PhysMemoryRegion;

/// Trait for common behavior across different types of page table entries.
pub trait PageTableEntry: Sized + Copy + Clone {
    /// Returns `true` if the entry is valid (V bit set).
    fn is_valid(self) -> bool;

    /// Returns the raw value of this page descriptor.
    fn as_raw(self) -> u64;

    /// Returns a representation of the page descriptor from a raw value.
    fn from_raw(v: u64) -> Self;

    /// Return a new invalid page descriptor.
    fn invalid() -> Self;
}

/// Trait for descriptors that can point to a next-level table.
pub trait TableMapper: PageTableEntry {
    /// Returns the physical address of the next-level table, if this descriptor
    /// is a table descriptor.
    fn next_table_address(self) -> Option<PA>;

    /// Creates a new descriptor that points to the given next-level table.
    fn new_next_table(pa: PA) -> Self;
}

/// A descriptor that maps a physical address (L0-L2 blocks and L3 page).
pub trait PaMapper: PageTableEntry {
    /// Constructs a new valid page descriptor that maps a physical address.
    fn new_map_pa(page_address: PA, memory_type: MemoryType, perms: PtePermissions) -> Self;

    /// Return how many bytes this descriptor type maps.
    fn map_shift() -> usize;

    /// Whether a subsection of the region could be mapped via this type of
    /// page.
    fn could_map(region: PhysMemoryRegion, va: VA) -> bool;

    /// Return the mapped physical address.
    fn mapped_address(self) -> Option<PA>;
}

#[derive(Clone, Copy)]
struct TableAddr(PA);

impl TableAddr {
    fn from_raw_ppn(ppn: u64) -> Self {
        Self(PA::from_value((ppn as usize) << PAGE_SHIFT))
    }
}

#[derive(Debug, Clone, Copy)]
pub enum MemoryType {
    Device,
    Normal,
}

// RISC-V Page Table Entry Bitfields
register_bitfields![u64,
    CommonFields [
        VALID     OFFSET(0) NUMBITS(1) [],
        READ      OFFSET(1) NUMBITS(1) [],
        WRITE     OFFSET(2) NUMBITS(1) [],
        EXECUTE   OFFSET(3) NUMBITS(1) [],
        USER      OFFSET(4) NUMBITS(1) [],
        GLOBAL    OFFSET(5) NUMBITS(1) [],
        ACCESSED  OFFSET(6) NUMBITS(1) [],
        DIRTY     OFFSET(7) NUMBITS(1) [],
        // Software defined bits in RSW (bits 8-9)
        COW       OFFSET(8) NUMBITS(1) [], 
        // PPN is bits 10-53
        PPN       OFFSET(10) NUMBITS(44) [],
        // Reserved/Pbmt bits 54-63
        PBMT      OFFSET(61) NUMBITS(2) [
            None = 0,
            // PMA = 0, // Removed duplicate discriminant
            NC = 1,
            IO = 2,
        ]
    ]
];

macro_rules! define_descriptor {
    (
        $(#[$outer:meta])*
        $name:ident,
        // Optional: Implement TableMapper if this section is present
        $( table: $can_table:literal, )?
        // Optional: Implement PaMapper if this section is present
        $( map: {
                shift: $tbl_shift:literal,
            },
        )?
    ) => {
        #[repr(transparent)]
        #[derive(Debug, Clone, Copy, PartialEq, Eq)]
        $(#[$outer])*
        pub struct $name(u64);

        impl PageTableEntry for $name {
            fn is_valid(self) -> bool { 
                let reg = InMemoryRegister::<u64, CommonFields::Register>::new(self.0);
                reg.is_set(CommonFields::VALID)
            }
            fn as_raw(self) -> u64 { self.0 }
            fn from_raw(v: u64) -> Self { Self(v) }
            fn invalid() -> Self { Self(0) }
        }

        $(
            impl TableMapper for $name {
                fn next_table_address(self) -> Option<PA> {
                     // Macro syntax requirement: use the captured variable
                     const _IS_TABLE: bool = $can_table; 

                     let reg = InMemoryRegister::<u64, CommonFields::Register>::new(self.0);
                     // A valid non-leaf PTE has V=1, and R=W=X=0
                     if reg.is_set(CommonFields::VALID)
                        && !reg.is_set(CommonFields::READ)
                        && !reg.is_set(CommonFields::WRITE)
                        && !reg.is_set(CommonFields::EXECUTE) 
                    {
                        let ppn = reg.read(CommonFields::PPN);
                        Some(TableAddr::from_raw_ppn(ppn).0)
                     } else {
                         None
                     }
                }

                fn new_next_table(pa: PA) -> Self {
                    let reg = InMemoryRegister::<u64, CommonFields::Register>::new(0);
                    let ppn = (pa.value() >> PAGE_SHIFT) as u64;
                    
                    reg.modify(CommonFields::VALID::SET 
                        + CommonFields::PPN.val(ppn));
                    
                    Self(reg.get())
                }
            }
        )?

        $(
            impl $name {
                /// Returns the interpreted permissions
                pub fn permissions(self) -> Option<PtePermissions> {
                    let reg = InMemoryRegister::<u64, CommonFields::Register>::new(self.0);
                    
                    if !reg.is_set(CommonFields::VALID) {
                        return None;
                    }

                    let r = reg.is_set(CommonFields::READ);
                    let w = reg.is_set(CommonFields::WRITE);
                    let x = reg.is_set(CommonFields::EXECUTE);
                    
                    if !r && !x {
                        return None;
                    }

                    let user = reg.is_set(CommonFields::USER);
                    let cow = reg.is_set(CommonFields::COW);

                    Some(PtePermissions::from_raw_bits(
                        true, 
                        w,    
                        x,    
                        user, 
                        cow,  
                    ))
                }

                pub fn set_permissions(self, perms: PtePermissions) -> Self {
                    let reg = InMemoryRegister::<u64, CommonFields::Register>::new(self.0);

                    if perms.is_user() { reg.modify(CommonFields::USER::SET); } 
                    else { reg.modify(CommonFields::USER::CLEAR); }

                    reg.modify(CommonFields::READ::SET);

                    if perms.is_write() { reg.modify(CommonFields::WRITE::SET); } 
                    else { reg.modify(CommonFields::WRITE::CLEAR); }

                    if perms.is_execute() { reg.modify(CommonFields::EXECUTE::SET); } 
                    else { reg.modify(CommonFields::EXECUTE::CLEAR); }

                    if perms.is_cow() { reg.modify(CommonFields::COW::SET); } 
                    else { reg.modify(CommonFields::COW::CLEAR); }
                    
                    Self(reg.get())
                }
            }

            impl PaMapper for $name {
                fn map_shift() -> usize { $tbl_shift }

                fn could_map(region: PhysMemoryRegion, va: VA) -> bool {
                    let is_aligned = |addr: usize| (addr & ((1 << $tbl_shift) - 1)) == 0;
                    is_aligned(region.start_address().value())
                        && is_aligned(va.value())
                        && region.size() >= (1 << $tbl_shift)
                }

                fn new_map_pa(page_address: PA, memory_type: MemoryType, perms: PtePermissions) -> Self {
                    let is_aligned = |addr: usize| (addr & ((1 << $tbl_shift) - 1)) == 0;
                    if !is_aligned(page_address.value()) {
                        panic!("Cannot map non-aligned physical address");
                    }

                    let reg = InMemoryRegister::<u64, CommonFields::Register>::new(0);
                    
                    let ppn = (page_address.value() >> PAGE_SHIFT) as u64;
                    reg.modify(CommonFields::PPN.val(ppn));
                    
                    reg.modify(CommonFields::VALID::SET 
                        + CommonFields::ACCESSED::SET 
                        + CommonFields::DIRTY::SET);

                    match memory_type {
                        MemoryType::Device => {
                             reg.modify(CommonFields::PBMT::IO);
                        }
                        MemoryType::Normal => {
                             reg.modify(CommonFields::PBMT::None);
                        }
                    }

                    Self(reg.get()).set_permissions(perms)
                }

                fn mapped_address(self) -> Option<PA> {
                    let reg = InMemoryRegister::<u64, CommonFields::Register>::new(self.0);
                    
                    if !reg.is_set(CommonFields::VALID) { return None; }

                    if !reg.is_set(CommonFields::READ) && !reg.is_set(CommonFields::EXECUTE) {
                        return None; 
                    }

                    let ppn = reg.read(CommonFields::PPN);
                    Some(PA::from_value((ppn as usize) << PAGE_SHIFT))
                }
            }
        )?
    };
}

define_descriptor!(
    /// A Level 0 descriptor. (Root in Sv48)
    L0Descriptor,
    table: true, 
    map: {
        shift: 39,     
    },
);

define_descriptor!(
    /// A Level 1 descriptor. 
    L1Descriptor,
    table: true,
    map: {
        shift: 30,     
    },
);

define_descriptor!(
    /// A Level 2 descriptor. 
    L2Descriptor,
    table: true,
    map: {
        shift: 21,    
    },
);

define_descriptor!(
    /// A Level 3 descriptor. The standard 4K Page.
    L3Descriptor,
    map: {
        shift: 12,    
    },
);

pub enum L3DescriptorState {
    Invalid,
    Swapped,
    Valid,
}

impl L3Descriptor {
    const SWAPPED_MASK: u64 = 1 << 63; 

    pub fn state(self) -> L3DescriptorState {
        if self.is_valid() {
            L3DescriptorState::Valid
        } else if (self.0 & Self::SWAPPED_MASK) != 0 {
            L3DescriptorState::Swapped
        } else {
            L3DescriptorState::Invalid
        }
    }

    pub fn mark_as_swapped(self) -> Self {
        let reg = InMemoryRegister::<u64, CommonFields::Register>::new(self.0);
        reg.modify(CommonFields::VALID::CLEAR);
        Self::from_raw(reg.get() | Self::SWAPPED_MASK)
    }
}