use crate::memory::{
    address::{PA, VA},
    permissions::PtePermissions,
    region::PhysMemoryRegion,
};
use bitflags::bitflags;

bitflags! {
    #[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
    pub struct PteFlags: u64 {
        const VALID = 1 << 0;
        const READ = 1 << 1;
        const WRITE = 1 << 2;
        const EXECUTE = 1 << 3;
        const USER = 1 << 4;
        const GLOBAL = 1 << 5;
        const ACCESSED = 1 << 6;
        const DIRTY = 1 << 7;
        const COW = 1 << 8;
    }
}

#[derive(Clone, Copy, Debug)]
#[repr(transparent)]
pub struct Sv39Descriptor(u64);

pub type PageDescriptor = Sv39Descriptor;

pub trait PageTableEntry: Copy {
    fn from_raw(raw: u64) -> Self;
    fn as_raw(self) -> u64;
    fn is_valid(self) -> bool;
    fn is_table(self) -> bool;
    fn mapped_address(self) -> Option<PA>;
    fn output_address(self) -> PA; 
}

impl Sv39Descriptor {
    pub fn new(pa: PA, flags: PteFlags) -> Self {
        let ppn = (pa.value() as u64) >> 12;
        Self((ppn << 10) | flags.bits())
    }
}

impl PageTableEntry for Sv39Descriptor {
    fn from_raw(raw: u64) -> Self { Self(raw) }
    fn as_raw(self) -> u64 { self.0 }
    
    fn is_valid(self) -> bool {
        (self.0 & PteFlags::VALID.bits()) != 0
    }

    fn is_table(self) -> bool {
        // V=1, R=0, W=0, X=0 implies next level table
        (self.0 & 0xF) == 1
    }

    fn mapped_address(self) -> Option<PA> {
        if self.is_valid() {
             Some(PA::from_value(((self.0 >> 10) & ((1 << 44) - 1)) as usize * 4096))
        } else {
            None
        }
    }
    
    fn output_address(self) -> PA {
        self.mapped_address().unwrap_or(PA::from_value(0))
    }
}

#[derive(Clone, Copy)]
pub enum MemoryType {
    Normal,
    Device,
}