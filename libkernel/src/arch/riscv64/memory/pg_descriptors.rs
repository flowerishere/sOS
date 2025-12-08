use bitflags::bitflags;
use crate::memory::address::PA;

bitflags! {
    /// RISC-V Page Table Entry Flags (Sv39 / Sv48 / Sv57)
    #[derive(Clone, Copy, Debug)]
    pub struct PteFlags: u64 {
        const VALID     = 1 << 0;
        const READ      = 1 << 1;
        const WRITE     = 1 << 2;
        const EXECUTE   = 1 << 3;
        const USER      = 1 << 4;
        const GLOBAL    = 1 << 5;
        const ACCESSED  = 1 << 6;
        const DIRTY     = 1 << 7;
        
        // RSW (Reserved for Software) bits 8-9
        const RSW_0     = 1 << 8;
        const RSW_1     = 1 << 9;
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum MemoryType {
    Normal,
    Device,
    // RISC-V Pbmt extension could be added here later
}

/// A RISC-V Page Table Entry.
#[derive(Clone, Copy, Debug)]
#[repr(transparent)]
pub struct PageDescriptor(u64);

impl PageDescriptor {
    pub fn new(pa: PA, flags: PteFlags) -> Self {
        // PPN is bits 10-53 (for Sv48)
        // PPN = PA >> 12
        let ppn = (pa.value() >> 12) as u64;
        let val = (ppn << 10) | flags.bits();
        Self(val)
    }

    pub fn invalid() -> Self {
        Self(0)
    }

    pub fn is_valid(&self) -> bool {
        self.0 & PteFlags::VALID.bits() != 0
    }

    pub fn is_table(&self) -> bool {
        // In RISC-V, a valid PTE is a pointer to the next level (Table)
        // if R=0, W=0, X=0.
        let flags = self.0 & 0x3FF;
        (flags & PteFlags::VALID.bits() != 0) &&
        (flags & (PteFlags::READ.bits() | PteFlags::WRITE.bits() | PteFlags::EXECUTE.bits()) == 0)
    }

    pub fn is_block(&self) -> bool {
        // It's a "Mega/Giga Page" (Leaf) if valid and R/W/X are set.
        let flags = self.0 & 0x3FF;
        (flags & PteFlags::VALID.bits() != 0) &&
        (flags & (PteFlags::READ.bits() | PteFlags::WRITE.bits() | PteFlags::EXECUTE.bits()) != 0)
    }

    pub fn output_address(&self) -> PA {
        // Extract PPN (bits 10-53) and convert to PA
        // Mask 44 bits for PPN in Sv48
        let ppn = (self.0 >> 10) & 0xFFF_FFFF_FFFF; 
        PA::from_value((ppn << 12) as usize)
    }
    
    pub fn raw(&self) -> u64 {
        self.0
    }
}