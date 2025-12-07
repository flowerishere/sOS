use bitflags::bitflags;
use libkernel::memory::address::PA;

bitflags! {
    ///RISC-V Table Entry Flags
    #[derive(Clone, Copy, Debug)]
    pub struct PteFlags: u64 {
        const VALID    = 1 << 0;  // PTE is valid
        const READ     = 1 << 1;  // PTE is readable
        const WRITE    = 1 << 2;  // PTE is writable
        const EXECUTE  = 1 << 3;  // PTE is executable
        const USER     = 1 << 4;  // User accessible
        const GLOBAL   = 1 << 5;  // Global mapping
        const ACCESSED = 1 << 6;  // Page has been accessed
        const DIRTY    = 1 << 7;  // Page has been written to
    
        //RSW(Reserved for Software)
        const RSW0     = 1 << 8;
        const RSW1     = 1 << 9;
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum MemoryType {
    Normal,
    Device,
    //RISC-V standard doesn't strictly distinguish Device/Normal in PTEs without Pbmt extension
    //For basic Sv48, we usually treat everything as Strong Ordered via PMA or use default attributes
    //If Pbmt is supported, we would add bits here(e.g., bit61, 62)
}

///RISC-V Page Table Entry
#[derive(Clone, Copy, Debug)]
#[repr(transparent)]
pub struct PageDescriptor(u64);

impl PageDescriptor {
    pub fn new(pa:PA, flags:PteFlags) -> Self {
        //PPN is bits 10-53(for Sv48/Sv39)
        //PPN = PA >> 12
        let ppn = (pa.value() >> 12) as u64;
        let val = (ppn << 10) | flags.bits();
        Self(val)
    }

    pub fn invalid() -> Self {
        Self(0)
    }

    pub fn is_valid(&self) -> bool {
        (self.0 & PteFlags::VALID.bits()) != 0
    }

    pub fn is_table(&self) -> bool {
        //a valid PTE is a pointer to the next level(Table)
        //if R = 0, W = 0, X = 0
        //if any of R/W/X is set, it's a leaf PTE
        let flags = self.0 & 0x3FF; //mask lower 10 bits
        (flags & PteFlags::VALID.bits()) != 0 &&
        (flags & (PteFlags::READ.bits() | PteFlags::WRITE.bits() | PteFlags::EXECUTE.bits()) == 0)
    }

    pub fn is_block(&self) -> bool {
        //a "mega/giga page" if valid and R/W/X are set
        let flags = self.0 & 0x3FF;
        (flags & PteFlags::VALID.bits()) != 0 &&
        (flags & (PteFlags::READ.bits() | PteFlags::WRITE.bits() | PteFlags::EXECUTE.bits()) != 0)
    }

    pub fn output_address(&self) -> PA {
        //extract PPN from bits 10-53 and convert to PA
        //mask for Sv48 PPN usually fits in u64, effectively((val >> 10) & PPN_MASK) << 12
        //Simplified:(val >> 10) << 12
        let ppn = (self.0 >> 10) & 0xFFFFFFFFFFF; //44 bits for Sv48
        PA::from_value((ppn << 12) as usize)
    }

    pub fn raw(&self) -> u64 {
        self.0
    }
}
