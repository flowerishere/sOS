use core::marker::PhantomData;
use core::cmp;

use crate::{
    error::Result,
    memory::{
        address::{PA, TPA, VA},
    },
};

use super::{
    pg_descriptors::{PageDescriptor, PteFlags},
    pg_tables::{MapAttributes, MappingContext, PgTable, PgTableArray, PageAllocator, PageTableMapper},
    tlb::TlbInvalidator,
};

pub struct Walk<T: PgTable> {
    phantom: PhantomData<T>,
}

impl<T: PgTable> Walk<T> {
    /// Maps a range of memory starting from the current table level.
    pub fn map_range<A, M, I>(
        table_pa: TPA<PgTableArray<T>>,
        attrs: MapAttributes,
        ctx: &mut MappingContext<A, M, I>,
    ) -> Result<()>
    where
        A: PageAllocator + ?Sized,
        M: PageTableMapper + ?Sized,
        I: TlbInvalidator + ?Sized,
    {
        Self::map_range_internal(
            table_pa,
            attrs.virt.start_address().value(),
            attrs.virt.end_address().value(),
            attrs.phys.start_address().value(),
            &attrs,
            ctx
        )
    }

    fn map_range_internal<A, M, I>(
        table_pa: TPA<PgTableArray<T>>,
        va_start: usize,
        va_end: usize,
        pa_start: usize,
        attrs: &MapAttributes,
        ctx: &mut MappingContext<A, M, I>,
    ) -> Result<()>
    where
        A: PageAllocator + ?Sized,
        M: PageTableMapper + ?Sized,
        I: TlbInvalidator + ?Sized,
    {
        // Stack buffer to store child tables that need recursion.
        let mut children_to_visit = [(0u16, PA::from_value(0)); 512];
        let mut children_count = 0;

        // Process current level in a separate scope to end borrows early
        {
            let allocator = &mut ctx.allocator;
            let invalidator = &ctx.invalidator;
            let mapper = &mut ctx.mapper;

            unsafe {
                mapper.with_page_table(table_pa, |table_va| -> Result<()> {
                    let table = table_va.as_ptr() as *mut PgTableArray<T>;
                    let page_size = T::page_size();
                    
                    let start_idx = T::index(va_start);
                    let end_idx = T::index(va_end - 1);

                    let mut current_va = va_start;
                    let mut current_pa = pa_start;

                    for i in start_idx..=end_idx {
                        let entry = &mut (*table).entries[i];
                        
                        let slot_boundary = (current_va & !(page_size - 1)) + page_size;
                        let chunk_end = cmp::min(va_end, slot_boundary);
                        let chunk_len = chunk_end - current_va;

                        if T::is_leaf() {
                            let flags = Self::make_flags(attrs.perms, attrs.mem_type);
                            let pte = PageDescriptor::new(PA::from_value(current_pa), flags);
                            
                            *entry = pte;
                            invalidator.invalidate_page(VA::from_value(current_va));
                        } else {
                            let child_pa = if entry.is_valid() && entry.is_table() {
                                entry.output_address()
                            } else {
                                let new_table = allocator.allocate_page_table::<T::NextLevel>()?;
                                *entry = PageDescriptor::new(new_table.to_untyped(), PteFlags::VALID);
                                new_table.to_untyped()
                            };

                            if children_count < 512 {
                                children_to_visit[children_count] = (i as u16, child_pa);
                                children_count += 1;
                            }
                        }

                        current_va += chunk_len;
                        current_pa += chunk_len;
                    }
                    Ok(())
                })??; 
            }
        } // Borrows end here

        // Recursion phase (Pass 2)
        // Now we can safely use ctx again for recursion
        for i in 0..children_count {
            let (idx, child_pa) = children_to_visit[i];
            let idx = idx as usize;
            let page_size = T::page_size();

            let start_idx = T::index(va_start);
            let idx_base_va = (va_start & !(page_size - 1)) + (idx - start_idx) * page_size;
            
            let chunk_start = cmp::max(va_start, idx_base_va);
            let chunk_end = cmp::min(va_end, idx_base_va + page_size);
            let chunk_pa = pa_start + (chunk_start - va_start);

            Walk::<T::NextLevel>::map_range_internal(
                TPA::from_value(child_pa.value()),
                chunk_start,
                chunk_end,
                chunk_pa,
                attrs,
                ctx  // 直接传递原始 ctx，不需要重构
            )?;
        }

        Ok(())
    }
    
    fn make_flags(perms: crate::memory::permissions::PtePermissions, _mt: super::pg_descriptors::MemoryType) -> PteFlags {
        let mut flags = PteFlags::VALID | PteFlags::ACCESSED | PteFlags::DIRTY;
        
        if perms.is_write() {
            flags |= PteFlags::WRITE | PteFlags::READ; 
        } else {
            flags |= PteFlags::READ;
        }
        
        if perms.is_execute() {
             flags |= PteFlags::EXECUTE;
        }
        
        if perms.is_user() {
            flags |= PteFlags::USER;
        } else {
            flags |= PteFlags::GLOBAL;
        }
        
        flags
    }
}