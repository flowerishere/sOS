use super::{
    exceptions::{TrapFrame, secondary_exceptions_init},
    memory::{fixmap::FIXMAPS, mmu::setup_kern_addr_space},
};
use crate::{
    arch::{ArchImpl, riscv64::exceptions::exceptions_init},
    console::setup_console_logger,
    drivers::{
        fdt_prober::{probe_for_fdt_devices, set_fdt_va},
        init::run_initcalls,
    },
    interrupts::{
        cpu_messenger::{Message, cpu_messenger_init, message_cpu},
        get_interrupt_root,
    },
    kmain,
    memory::{INITAL_ALLOCATOR, PAGE_ALLOC},
    sched::{sched_init_secondary, uspc_ret::dispatch_userspace_task},
};
use riscv::{
    asm,
    register::{sstatus, sie, stevc, satp},
};
use alloc::string::ToString;
use core::arch::global_asm;
use libkernel::{
    CpuOps,
    arch::riscv64::memory::pg_tables::{L0Table, PgTableArray},
    error::Result,
    memory::{
        address::{PA, TPA, VA},
        page_alloc::FrameAllocator,
    },
    sync::per_cpu::setup_percpu,
};
use logical_map::setup_logical_map;
use memory::{setup_allocator, setup_stack_and_heap};
use secondary::{boot_secondaries, cpu_count, save_satp, secondary_booted};

mod logical_map;
mod memory;
mod paging_bootstrap;
mod secondary;

global_asm!(include_str!("start.s"));

/// Stage 1 Initialize of the system architechture.
///
/// This function is called by the main primary CPU with the other CPUs parked.
/// All interrupts should be disabled, the ID map setup in SATP and the highmem
/// map setup in the kernel page tables.
///
/// The memory map is setup as follows:
///
/// 0xffff_8000_0000_0000 | kernel image & Direct Map Base(physmap)
/// 0xffff_9000_0000_0000 | Fixed mappings
/// 0xffff_b000_0000_0000 | Kernel Heap
/// 0xffff_b800_0000_0000 | Kernel Stack (per CPU)
/// 0xffff_d000_0000_0000 | MMIO remap region
/// 0xffff_e000_0000_0000 | Exception vector trampoline(high memory)
/// 
/// Returns the stack pointer in A0, which should be set by the boot asm.
#[unsafe(no_mangle)]
fn arch_init_stage1(
    dtb_ptr: TPA<u8>,
    image_start:PA,
    image_end:PA,
    // Start.s passes the satp value or root table PA.
    //accept the Typed Physical Address of the root table to match common style.
    highmem_pgtable_base: TPA<PgTableArray<L0Table>>,
) -> VA {
    (|| -> Result<VA> {
        setup_console_logger();
        setup_allocator(dtb_addr, image_start, image_end)?;
        let dtb_addr = {
            let mut fixmaps = FIXMAPS.lock_save_irq();
            fixmaps.setup_fixmaps(highmem_pgtable_base);
            unsafe { fixmaps.remap_fdt(dtb_ptr) }.unwrap()
        };
        set_fdt_va(dtb_addr.cast());

        //setup the linear mapping (Physmap)
        setup_logical_map(highmem_pgtable_base)?;

        //setup the kernel stack and heap
        let stack_addr = setup_stack_and_heap(highmem_pgtable_base)?;

        //setup global kernel address space management
        setup_kern_addr_space(highmem_pgtable_base)?;

        Ok(stack_addr)
    })()
    .unwrap_or_else(|_| park_cpu())
}

#[unsafe(no_mangle)]
fn arch_init_stage2(frame: *mut TrapFrame) -> *mut TrapFrame {
    //save the satp(root page table) for booting secondary cpus
    //In RISC-V, secondaries need the SATP to enable paging
    save_satp(satp::read().bits());
    //unmap the identity mapping here
    //rely on the logic that we are executing in high memory
    asm::sfence_vma_all();

    //setup to switch to the real page allocator
    let smalloc = INITAL_ALLOCATOR
        .lock_save_irq()
        .take()
        .expect("Smalloc should not have been taken yet");

    let page_alloc = unsafe { FrameAllocator::init(smalloc) };

    if PAGE_ALLOC.set(page_alloc).is_err() {
        panic!("Cannot setup physical memory allocator");
    }

    //Enable floating point(Status: FS = Initial)
    unsafe {
        sstatus::set_fs(sstatus::FS::Initial);
    }

    exceptions_init().expect("Failed to initialize exceptions");

    //Enable interrupts(SIE bit in sstatus)
    ArchImpl::enable_interrupts();

    unsafe { run_initcalls() };
    probe_for_fdt_devices();

    unsafe { setup_percpu(cpu_count()) };

    kmain(
        "--init=/bin/bash --rootfs=fat32fs --automount=/dev,devfs".to_string(),
        frame.cast(),//cast to generic context if needed, or update kmain signature
    );

    boot_secondaries();

    //Prove that we can send IPIs through the messenger.
    let _ = message_cpu(1, Message::Ping(ArchImpl::id() as _));

    frame
}

#[allow(dead_code)]//called from assembly or secondary boot logic
fn arch_init_secondary(ctx_frame: *mut TrapFrame) -> *mut TrapFrame {
    // Invalidate TLB to ensure clean state
    asm::sfence_vma_all();

    // Enable interrupts and exceptions.
    secondary_exceptions_init();

    if let Some(ic) = get_interrupt_root() {
        ic.enable_core(ArchImpl::id());
    }

    ArchImpl::enable_interrupts();

    secondary_booted();

    sched_init_secondary();

    dispatch_userspace_task(ctx_frame.cast());

    ctx_frame
}

#[unsafe(no_mangle)]
pub extern "C" fn park_cpu() -> ! {
    loop {
        unsafe {
            riscv::asm::wfi();
        }
    }
}