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
    //defined in start.S
    static exception_return: u8;
}
//data passed to the secondary core by the primary core via memory
//In SBI 'hart_start', this structure's physical opaque argument(a1)

#[derive(Debug)]
#[repr(C)]
struct SecondaryBootInfo {
    kstack_addr: VA,    //0:Virtual address of the top of the Kernel Stack
    satp: usize,        //8:SATP value for the kernel page table(PPN + mode)
    start_fn: VA,       //16:Virtual address of the Rust entry point(arch_init_secondary)
    exception_ret: VA,  //24:Virtual address of the trap return stub(set as RA)
}

/// entry point for secondary cores
/// register state on entry(provided by SBI)
/// a0: hartid
/// a1: opaque(physical address of SecondaryBootInfo)
#[unsafe(naked)]
#[unsafe(no_mangle)]
extern "C" fn do_secondary_start(hartid: usize, boot_info_ptr: *const SecondaryBootInfo) {
    naked_asm!(
        //enable MMU
        //load satp from secondarybootinfo(offset 8)
        "ld t0, 8(a1)",
        "csrw satp, t0",
        "sfence.vma", //flush TLBs
        
        //MMU is enabled, now we can use virtual addresses
        //setup stack pointer
        "ld sp, 0(a1)", //load kstack_addr from secondarybootinfo
        
        //setup context switch frame
        //reserve space for trapframe(34 * 8 = 272 bytes, align to 16 bytes -> 288 bytes)
        //matching start.s allocation size(288 bytes)
        "addi sp, sp, -288",
        
        //prepare arguments for rust entry
        //fn arch_init_secondary(ctx_frame: *mut TrapFrame)
        "mv a0, sp",          //arg0:ctx_frame

        //setup return address
        "ld ra, 24(a1)",      //load exception_ret from secondarybootinfo(offset 24)

        //jump to rust
        "ld t1, 16(a1)",      //load start_fn from secondarybootinfo(offset 16)
        "jr t1",               //jump to start_fn

        //should not reach here
        "wfi",
        "j .",
    )
}

fn prepare_for_secondary_entry() -> Result<(PA, PA)> {
    static mut SECONDARY_BOOT_CTX: MaybeUninit<SecondaryBootInfo> = MaybeUninit::uninit();

    //calculate physical address for the entry function and the context structure
    let entry_fn_pa = kfunc_pa!(do_secondary_start as *const () as usize);
    let ctx_pa = ksym_pa!(SECONDARY_BOOT_CTX);

    //allocate a new kernel stack for the secondary core
    let kstack_vaddr = allocate_kstack_region();
    let kstack_paddr = PAGE_ALLOC
        .get()
        .unwrap()
        .alloc_frames(KERNEL_STACK_PG_ORDER as _)?
        .leak();

    //map the stack into the kernel address space
    ArchImpl::kern_address_space().lock_save_irq().map_normal(
        kstack_paddr,
        kstack_vaddr,
        PtePermissions::rw(false),
    )?;

    //fill the boot context structure
    unsafe {
        (&raw mut SECONDARY_BOOT_CTX as *mut SecondaryBootInfo).write(SecondaryBootInfo {
            kstack_addr: kstack_vaddr.end_address(),
            satp: *SATP_VAL
                .get()
                .ok_or(KernelError::Other("SATP value not set"))?,
            start_fn: VA::from_value(arch_init_secondary as *const () as usize),
            exception_ret: VA::from_value(&exception_return as *const _ as usize),
        })
    };
    Ok((entry_fn_pa, ctx_pa))
}

fn do_boot_secondary(cpu_node: fdt_parser::Node<'static>) -> Result<()> {
    //parse "reg" property to get hart ID
    let id = cpu_node
        .reg()
        .and_then(|mut x| x.next().map(|x| x.address))
        .ok_or(KernelError::Other("reg property missing on CPU node"))?;

    // Skip boot core.
    if id as usize == ArchImpl::cpu_id() {
        return Ok(());
    }

    //check status property, skip if disabled
    if let Some(status) = cpu_node.find_property("status").map(|p| p.str()) {
        if status != "okay" && status != "ok" {
            return Ok(());
        }
    }

    //prepare stack and context
    let (entry_fn, ctx) = prepare_for_secondary_entry()?;

    //reset the handshake flag
    SECONDARY_BOOTED.store(false, Ordering::Relaxed);

    //call SBI hart_start to boot the secondary core
    match sbi_rt::hart_start(id as usize, entry_fn_pa.value(), ctx_pa.value()) {
        sbi_rt::SbiRet { error:0, .. } => Ok(()),
        sbi_rt::SbiRet { error, .. } => {
            warn!("SBI hart_start failed for hart {} with error code {}", id, error);
            return Err(KernelError::IO);
        }
    }

    //wait for the secondary core to signal it has booted
    let timeout = now().map(|x| x + Duration::from_millis(100));
    while !SECONDARY_BOOTED.load(Ordering::Acquire) {
        spin_loop();
        if let Some(timeout) = timeout
            && let Some(now) = now()
            && now >= timeout
        {
            return Err(KernelError::Other("timeout waiting for core entry"));
        }
    }
    Ok(())
}

fn cpu_node_iter() -> impl Iterator<Item = fdt_parser::Node<'static>> {
    let fdt = get_fdt();

    fdt.all_nodes().filter(|node| {
        node.find_property("device_type")
            .map(|prop| prop.str() == "cpu")
            .unwrap_or(false)
    })
}

pub fn cpu_count() -> usize {
    cpu_node_iter().count()
}

// saves the SATP value of the kernel page table for secondary cores to use
pub fn save_satp(val: usize) {
    if SATP_VAL.set(val).is_err() {
        warn!("Attempted to set SATP value multiple times");
    }
}

// called by secondary cores to signal they have booted
pub fn secondary_booted() {
    let id = ArchImpl::cpu_id();
    info!("CPU {} online", id);
    SECONDARY_BOOTED.store(true, Ordering::Release);
}

//stores the SATP value for secondary cores to enable MMU
static SATP_VAL: OnceLock<usize> = OnceLock::new();

//synchronization flag to serialize secondary booting
static SECONDARY_BOOTED: AtomicBool = AtomicBool::new(false);