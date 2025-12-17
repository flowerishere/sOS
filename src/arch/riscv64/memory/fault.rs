use core::mem;
use alloc::boxed::Box;
use crate::{
    arch::riscv64::{
        memory::uaccess::{UACESS_ABORT_DEFERRED, UACESS_ABORT_DENIED},
        TrapFrame,
    },
    memory::fault::{FaultResolution, handle_demand_fault, handle_protection_fault},
    sched::{current_task, spawn_kernel_work},
};
use libkernel::{
    UserAddressSpace,
    error::Result,
    memory::{address::VA, proc_vm::vmarea::AccessKind, region::VirtMemoryRegion},
};
use riscv::register::scause::{self, Exception};

#[repr(C)]
struct FixupTable {
    start: VA,
    end: VA,
    fixup: VA,
}

unsafe extern "C" {
    static __UACCESS_FIXUP: FixupTable;
}

impl FixupTable {
    fn is_in_fixup(&self, addr: VA) -> bool {
        VirtMemoryRegion::from_start_end_address(self.start, self.end).contains_address(addr)
    }
}

pub fn handle_page_fault(stval: usize, cause: Exception, tf: &mut TrapFrame) -> Result<()> {
    let fault_addr = VA::from_value(stval);
    let access_kind = match cause {
        Exception::InstructionPageFault => AccessKind::Execute,
        Exception::LoadPageFault => AccessKind::Read,
        Exception::StorePageFault => AccessKind::Write,
        _ => panic!("handle_page_fault called with non-fault cause"),
    };

    // 检查特权级：如果触发异常时是 User Mode (SPP=0)，则是普通用户缺页
    // sstatus.SPP 在 TrapFrame 中通常需要手动保存或通过 sstatus 判断
    // RISC-V sstatus SPP 位是 bit 8. 0 = User, 1 = Supervisor.
    let is_kernel_fault = (tf.sstatus & (1 << 8)) != 0;

    if is_kernel_fault {
        handle_kernel_mem_fault(fault_addr, access_kind, tf);
        return Ok(());
    }

    // 用户态缺页处理
    match run_mem_fault_handler(fault_addr, access_kind) {
        Ok(FaultResolution::Resolved) => Ok(()),
        Ok(FaultResolution::Denied) => {
            panic!("SIGSEGV: Process {} accessed {:?} at {:x}", 
                current_task().process.tgid, access_kind, fault_addr.value());
        },
        Ok(FaultResolution::Deferred(fut)) => {
            spawn_kernel_work(async {
                if Box::into_pin(fut).await.is_err() {
                    panic!("Deferred page fault failed");
                }
            });
            Ok(())
        }
        Err(e) => Err(e),
    }
}

fn run_mem_fault_handler(fault_addr: VA, access_kind: AccessKind) -> Result<FaultResolution> {
    let task = current_task();
    let mut vm = task.vm.lock_save_irq();

    let pg_info = vm.mm().address_space().translate(fault_addr);

    match pg_info {
        Some(info) => handle_protection_fault(&mut vm, fault_addr, access_kind, info),
        None => handle_demand_fault(&mut vm, fault_addr, access_kind),
    }
}

fn handle_kernel_mem_fault(fault_addr: VA, access_kind: AccessKind, tf: &mut TrapFrame) {
    let pc = VA::from_value(tf.sepc);
    if unsafe { __UACCESS_FIXUP.is_in_fixup(pc) } {
        handle_uaccess_abort(fault_addr, access_kind, tf);
        return;
    }

    panic!("Kernel memory fault at {:#x}, addr={:#x}. Context: {:?}", tf.sepc, fault_addr.value(), tf);
}

fn handle_uaccess_abort(fault_addr: VA, access_kind: AccessKind, tf: &mut TrapFrame) {
    match run_mem_fault_handler(fault_addr, access_kind) {
        Ok(FaultResolution::Resolved) => (),
        Ok(FaultResolution::Denied) => {
            tf.regs[10] = UACESS_ABORT_DENIED; // a0 = 1
            tf.sepc = unsafe { __UACCESS_FIXUP.fixup.value() };
        }
        
        Ok(FaultResolution::Deferred(fut)) => {
            let ptr = Box::into_raw(fut);
            let (data_ptr, vtable_ptr): (usize, usize) = unsafe { mem::transmute(ptr) };

            tf.regs[10] = UACESS_ABORT_DEFERRED; // a0 = 2
            tf.regs[11] = data_ptr;              // a1 = future ptr
            tf.regs[13] = vtable_ptr;            // a3 = vtable ptr
            
            tf.sepc = unsafe { __UACCESS_FIXUP.fixup.value() };
        }
        
        Err(_) => panic!("UAccess fault handler internal error"),
    }
}