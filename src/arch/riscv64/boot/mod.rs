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
    register::{sstatus, sie, stvec, satp},
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

global_asm!(include_str!("start.S"));
// ==================== 调试辅助函数 ====================

// 直接写 UART 寄存器 (0x1000_0000)，不经过 SBI，避免中断噪音
fn early_putchar(c: u8) {
    unsafe {
        // UART_BASE = 0x1000_0000
        // THR (Transmitter Holding Register) 是偏移 0 的寄存器
        // LSR (Line Status Register) 是偏移 5 的寄存器
        let uart_base = 0x1000_0000 as *mut u8;
        let lsr = uart_base.add(5);
        
        // 等待发送缓冲区为空 (LSR bit 5: THRE)
        // 这一步在 QEMU 中其实不是必须的，但在真实硬件上很重要
        // 为防死循环，我们在 QEMU 调试时可以简单地直接写
        core::ptr::write_volatile(uart_base, c);
    }
}

fn early_print(s: &str) {
    for c in s.bytes() {
        early_putchar(c);
    }
}
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
    image_start: PA,
    image_end: PA,
    highmem_pgtable_base: TPA<PgTableArray<L0Table>>,
) -> VA {
    (|| -> Result<VA> {
        early_print("\n>>> Stage 1 Start\n");

        setup_console_logger();
        
        early_print("> Setup Allocator...\n");
        setup_allocator(dtb_ptr, image_start, image_end)?;
        early_print("> Allocator OK\n");

        let dtb_addr = {
            early_print("> Locking Fixmaps...\n");
            let mut fixmaps = FIXMAPS.lock_save_irq();
            
            early_print("> Calling setup_fixmaps...\n");
            // 这里是高危点
            fixmaps.setup_fixmaps(highmem_pgtable_base);
            early_print("> setup_fixmaps returned\n");
            
            early_print("> Remapping FDT...\n");
            unsafe { fixmaps.remap_fdt(dtb_ptr) }.unwrap()
        };
        set_fdt_va(dtb_addr.cast());

        early_print("> Setup Logical Map...\n");
        setup_logical_map(highmem_pgtable_base)?;

        early_print("> Setup Stack/Heap...\n");
        let stack_addr = setup_stack_and_heap(highmem_pgtable_base)?;

        early_print("> Setup Kern Addr Space...\n");
        setup_kern_addr_space(highmem_pgtable_base)?;

        early_print(">>> Stage 1 Done\n");
        Ok(stack_addr)
    })()
    .unwrap_or_else(|e| {
        // 打印一点错误码提示（虽然此时还没有格式化输出）
        early_print("\n!!! FATAL ERROR in Stage 1 !!!\n");
        park_cpu()
    })
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