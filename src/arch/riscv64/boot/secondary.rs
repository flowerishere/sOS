use crate::{
    arch::{
        ArchImpl,
        riscv64::boot::{
            arch_init_secondary,
            memory::{KERNEL_STACK_PG_ORDER, allocate_kstack_region},
        },
    },
    drivers::{fdt_prober::get_fdt, timer::now},
    kfunc_pa, ksym_pa,
    memory::PAGE_ALLOC,
    sync::OnceLock,
};
use core::{
    arch::naked_asm,
    hint::spin_loop,
    mem::MaybeUninit,
    sync::atomic::{AtomicBool, Ordering},
    time::Duration,
};
use libkernel::{
    CpuOps, KernAddressSpace, VirtualMemory,
    error::{KernelError, Result},
    memory::{
        address::{PA, VA},
        permissions::PtePermissions,
    },
};
use log::{info, warn};

unsafe extern "C" {
    // defined in start.S
    static exception_return: u8;
}

// Data passed to the secondary core by the primary core via memory.
// In SBI 'hart_start', this structure's physical address is passed as the opaque argument (a1).
#[derive(Debug)]
#[repr(C)]
struct SecondaryBootInfo {
    kstack_addr: VA,    // 0: Virtual address of the top of the Kernel Stack
    satp: usize,        // 8: SATP value for the kernel page table (PPN + mode)
    start_fn: VA,       // 16: Virtual address of the Rust entry point (arch_init_secondary)
    exception_ret: VA,  // 24: Virtual address of the trap return stub (set as RA)
}

/// Entry point for secondary cores (Assembly Stub).
/// Register state on entry (provided by SBI):
/// a0: hartid
/// a1: opaque (physical address of SecondaryBootInfo)
#[unsafe(naked)]
#[unsafe(no_mangle)]
extern "C" fn do_secondary_start(hartid: usize, boot_info_ptr: *const SecondaryBootInfo) {
    naked_asm!(
        // 1. Enable MMU
        // Load satp from SecondaryBootInfo (offset 8)
        "ld t0, 8(a1)",
        "csrw satp, t0",
        "sfence.vma", // Flush TLBs to ensure the new mapping takes effect

        // --- MMU is enabled, now we are in Virtual Address space ---

        // 2. Setup Stack Pointer
        // Load kstack_addr from SecondaryBootInfo (offset 0)
        "ld sp, 0(a1)",
        
        // 3. Setup Context Switch Frame
        // Reserve space for TrapFrame (align to 16 bytes). 
        // Note: Ensure this size matches what arch_init_secondary expects or the TrapFrame definition.
        // Assuming 288 bytes here as per your snippet.
        "addi sp, sp, -288",
        
        // 4. Prepare arguments for Rust entry
        // fn arch_init_secondary(ctx_frame: *mut TrapFrame)
        "mv a0, sp",          // arg0: ctx_frame pointer (current sp)

        // 5. Setup Return Address
        // Load exception_ret from SecondaryBootInfo (offset 24)
        "ld ra, 24(a1)",      

        // 6. Jump to Rust Code
        // Load start_fn from SecondaryBootInfo (offset 16)
        "ld t1, 16(a1)",      
        "jr t1",              

        // Should not reach here
        "wfi",
        "j .",
    )
}

/// Prepares the boot context and stack for a single secondary core.
/// Returns the Physical Address (PA) of the entry function and the context structure.
fn prepare_for_secondary_entry() -> Result<(PA, PA)> {
    // We use a static mutable variable to store the boot info.
    // Since we boot cores sequentially (waiting for one to come online before starting the next),
    // it is safe to reuse this memory location.
    static mut SECONDARY_BOOT_CTX: MaybeUninit<SecondaryBootInfo> = MaybeUninit::uninit();

    // Calculate physical addresses for the entry function and the context structure
    // kfunc_pa! and ksym_pa! macros convert Kernel VA to PA.
    let entry_fn_pa = kfunc_pa!(do_secondary_start as *const () as usize);
    let ctx_pa = ksym_pa!(SECONDARY_BOOT_CTX);

    // Allocate a new kernel stack for the secondary core
    let kstack_vaddr = allocate_kstack_region();
    let kstack_paddr = PAGE_ALLOC
        .get()
        .unwrap()
        .alloc_frames(KERNEL_STACK_PG_ORDER as _)?
        .leak();

    // Map the stack into the kernel address space
    // We lock the kernel address space to safely modify the page table.
    ArchImpl::kern_address_space().lock_save_irq().map_normal(
        kstack_paddr,
        kstack_vaddr,
        PtePermissions::rw(false), // RW, Supervisor only
    )?;

    // Fill the boot context structure
    // We use write to safely initialize the MaybeUninit
    unsafe {
        (&raw mut SECONDARY_BOOT_CTX as *mut SecondaryBootInfo).write(SecondaryBootInfo {
            kstack_addr: kstack_vaddr.end_address(), // Stack grows down, so give the end address
            satp: *SATP_VAL
                .get()
                .ok_or(KernelError::Other("SATP value not set"))?,
            start_fn: VA::from_value(arch_init_secondary as *const () as usize),
            exception_ret: VA::from_value(&exception_return as *const _ as usize),
        })
    };

    Ok((entry_fn_pa, ctx_pa))
}

/// Boots a single secondary core described by `cpu_node`.
fn do_boot_secondary(cpu_node: fdt_parser::Node<'static>) -> Result<()> {
    // Parse "reg" property to get hart ID
    let id = cpu_node
        .reg()
        .and_then(|mut x| x.next().map(|x| x.address))
        .ok_or(KernelError::Other("reg property missing on CPU node"))?;

    // Skip the boot core (current core).
    if id as usize == ArchImpl::id() {
        return Ok(());
    }

    // Check "status" property, skip if disabled
    if let Some(status) = cpu_node.find_property("status").map(|p| p.str()) {
        if status != "okay" && status != "ok" {
            return Ok(());
        }
    }

    info!("Starting Hart ID: {}", id);

    // Prepare stack and context
    let (entry_fn_pa, ctx_pa) = prepare_for_secondary_entry()?;

    // Reset the handshake flag
    SECONDARY_BOOTED.store(false, Ordering::Relaxed);

    // Call SBI hart_start to boot the secondary core
    // Note: sbi_rt::hart_start takes (hartid, start_addr, opaque)
    match sbi_rt::hart_start(id as usize, entry_fn_pa.value(), ctx_pa.value()) {
        sbi_rt::SbiRet { error: 0, .. } => {
            // Success
        },
        sbi_rt::SbiRet { error, .. } => {
            warn!("SBI hart_start failed for hart {} with error code {}", id, error);
            return Err(KernelError::Other("SBI hart_start failed"));
        }
    }

    // Wait for the secondary core to signal it has booted
    // We give it a timeout (e.g., 100ms) to avoid hanging the boot process forever.
    let timeout = now().map(|x| x + Duration::from_millis(100));
    
    while !SECONDARY_BOOTED.load(Ordering::Acquire) {
        spin_loop();
        if let Some(timeout) = timeout
            && let Some(now) = now()
            && now >= timeout
        {
            warn!("Timeout waiting for Hart {} to come online", id);
            return Err(KernelError::Other("timeout waiting for core entry"));
        }
    }
    
    Ok(())
}

/// Main entry point to boot all secondary cores found in the FDT.
/// This is called from the primary core's boot sequence.
pub fn boot_secondaries() {
    info!("Detecting and booting secondary cores...");
    
    for node in cpu_node_iter() {
        if let Err(e) = do_boot_secondary(node) {
            // We log the error but continue trying to boot other cores.
            warn!("Failed to boot secondary core: {:?}", e);
        }
    }
    
    info!("Secondary core boot sequence finished. Total CPUs: {}", cpu_count());
}

/// Iterator over all "/cpus/cpu" nodes in the FDT.
fn cpu_node_iter() -> impl Iterator<Item = fdt_parser::Node<'static>> {
    let fdt = get_fdt();

    fdt.all_nodes().filter(|node| {
        node.find_property("device_type")
            .map(|prop| prop.str() == "cpu")
            .unwrap_or(false)
    })
}

/// Returns the total number of CPU nodes found in the FDT.
pub fn cpu_count() -> usize {
    cpu_node_iter().count()
}

/// Saves the SATP value of the kernel page table.
/// This must be called by the primary core before booting secondaries.
pub fn save_satp(val: usize) {
    if SATP_VAL.set(val).is_err() {
        warn!("Attempted to set SATP value multiple times");
    }
}

/// Called by secondary cores to signal they have initialized successfully.
pub fn secondary_booted() {
    let id = ArchImpl::id();
    info!("CPU {} online and synchronized", id);
    SECONDARY_BOOTED.store(true, Ordering::Release);
}

// Stores the SATP value (Page Table Root) for secondary cores to enable MMU.
static SATP_VAL: OnceLock<usize> = OnceLock::new();

// Synchronization flag to serialize secondary booting.
// Used as a handshake between the primary core (waiting) and the booting secondary core (signaling).
static SECONDARY_BOOTED: AtomicBool = AtomicBool::new(false);