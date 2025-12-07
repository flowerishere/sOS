use core::marker::PhantomData;

use libkernel::{
    error::{KernelError, Result},
    memory::{
        PAGE_SIZE,
        address::{PA, TPA},
        permissions::AccessPermissions,
    },
};

use super::{
    pg_descriptors::{PageDescriptor, PteFlags},
    pg_tables::{L3Table, MapAttributes, MappingContext, PgTable, PgTableArray},
};

pub struct Walk<T: PgTable> {
    phantom: PhantomData<T>,
}

impl<T: PgTable> Walk<T> {
    /// Maps a range of memory starting from the current table level.
    pub fn map_range(
        table_pa: TPA<PgTableArray<T>>,
        attrs: MapAttributes,
        ctx: &mut MappingContext,
    ) -> Result<()> {
        unsafe {
            ctx.mapper.with_page_table(table_pa, |table_va| {
                let table = table_va.as_mut_ptr();
                
                let start = attrs.virt.start_address().value();
                let end = attrs.virt.end_address().value();
                let page_size = T::page_size();

                // Align iteration to this level's page size
                let start_idx = T::index(start);
                // Calculate end index strictly.
                let end_idx = T::index(end - 1); 

                for i in start_idx..=end_idx {
                    let entry = &mut (*table).entries[i];
                    
                    // Calculate the VA for the start of this entry's coverage
                    let entry_va_start = if i == start_idx {
                        start
                    } else {
                        0 // Placeholder, logic handled below
                    };

                    // Check if we are at the target Leaf level
                    if T::is_leaf() {
                        Self::map_leaf(entry, attrs, ctx, i, start, end)?;
                    } else {
                        // We are at an intermediate table (L0, L1, L2)
                        // We need to ensure a child table exists.
                        
                        let child_pa = if entry.is_valid() && entry.is_table() {
                            entry.output_address()
                        } else {
                            // Allocate new table
                            let new_table = ctx.allocator.allocate_page_table::<T::NextLevel>()?;
                            // Link it: V=1, R/W/X=0 (Directory)
                            *entry = PageDescriptor::new(new_table.to_untyped(), PteFlags::VALID);
                            new_table.to_untyped()
                        };

                        Walk::<T::NextLevel>::map_range(
                            TPA::from_pa(child_pa),
                            attrs,
                            ctx
                        )?;
                    }
                }
                Ok(())
            })?
        }
    }

    fn map_leaf(
        entry: &mut PageDescriptor,
        attrs: MapAttributes,
        ctx: &mut MappingContext,
        idx: usize,
        walk_start: usize,
        walk_end: usize,
    ) -> Result<()> {     
        let level_size = T::page_size();
        Err(KernelError::NotImplemented) 
    }
}

// Redefining Walk to be robust
impl<T: PgTable> Walk<T> {
     pub fn map_range(
        table_pa: TPA<PgTableArray<T>>,
        attrs: MapAttributes,
        ctx: &mut MappingContext,
    ) -> Result<()> {
        // maintain the "current VA" context.
        Self::map_range_internal(table_pa, attrs.virt.start_address().value(), attrs.virt.end_address().value(), attrs.phys.start_address().value(), &attrs, ctx)
    }

    fn map_range_internal(
        table_pa: TPA<PgTableArray<T>>,
        va_start: usize, // The VA we are currently mapping in this call
        va_end: usize,
        pa_start: usize, // The PA corresponding to va_start
        attrs: &MapAttributes,
        ctx: &mut MappingContext,
    ) -> Result<()> {
        unsafe {
            ctx.mapper.with_page_table(table_pa, |table_va| {
                let table = table_va.as_mut_ptr();
                
                let page_size = T::page_size();
                
                // Calculate indices for the *current range* [va_start, va_end)
                let start_idx = T::index(va_start);
                let end_idx = T::index(va_end - 1);

                let mut current_va = va_start;
                let mut current_pa = pa_start;

                for i in start_idx..=end_idx {
                    let entry = &mut (*table).entries[i];
                    
                    if T::is_leaf() {
                        // Leaf Level (L3, 4KB)
                        // Create the PTE.
                        let flags = Self::make_flags(attrs.perms, attrs.mem_type);
                        let pte = PageDescriptor::new(PA::from_value(current_pa), flags);
                        
                        // Check if overwriting?
                        // if entry.is_valid() { ... }
                        
                        *entry = pte;
                        ctx.invalidator.invalidate_page(VA::from_value(current_va));
                        
                        current_va += page_size;
                        current_pa += page_size;
                    } else {
                        // Intermediate Level
                        // We must recurse.
                        
                        // Ensure child table exists
                        let child_pa = if entry.is_valid() && entry.is_table() {
                            entry.output_address()
                        } else {
                             let new_table = ctx.allocator.allocate_page_table::<T::NextLevel>()?;
                             *entry = PageDescriptor::new(new_table.to_untyped(), PteFlags::VALID);
                             new_table.to_untyped()
                        };
                        
                        let dist = page_size - (current_va & (page_size - 1));
                        let next_boundary = core::cmp::min(current_va + dist, va_end);
                        
                        let chunk_size = next_boundary - current_va;
                        
                        Walk::<T::NextLevel>::map_range_internal(
                            TPA::from_pa(child_pa),
                            current_va,
                            next_boundary,
                            current_pa,
                            attrs,
                            ctx
                        )?;
                        
                        current_va += chunk_size;
                        current_pa += chunk_size;
                    }
                }
                Ok(())
            })?
        }
    }
    
    fn make_flags(perms: libkernel::memory::permissions::PtePermissions, _mt: super::pg_descriptors::MemoryType) -> PteFlags {
        let mut flags = PteFlags::VALID | PteFlags::ACCESSED | PteFlags::DIRTY;
        
        // RISC-V: R, W, X
        // Kernel: R=1, W=1, X=0 (RW) or R=1, W=0, X=1 (RX)
        // User: U=1
        
        // Mapping PtePermissions (generic) to RISC-V
        // PtePermissions usually has `is_writable`, `is_executable`, `is_user`.
        
        if perms.is_writable() {
            flags |= PteFlags::WRITE | PteFlags::READ; // Write implies Read usually
        } else {
            flags |= PteFlags::READ;
        }
        
        if perms.is_executable() {
             flags |= PteFlags::EXECUTE;
             // Execute-only is possible in RISC-V but uncommon for kernel data, usually RX.
             // If write is set, RWX.
        }
        
        // If not executable and not writable, it's RO.
        
        if !perms.is_kernel_only() {
            flags |= PteFlags::USER;
        } else {
            // Kernel Global mapping
            flags |= PteFlags::GLOBAL;
        }
        
        flags
    }
}