use core::arch::global_asm;
use riscv::register::scause::{self, Trap};
use riscv::interrupt::{Exception, Interrupt};

global_asm!(include_str!("entry.S"));

#[repr(C)]
#[derive(Debug, Clone, Copy)]
pub struct TrapFrame {
    pub regs: [usize; 32],
    pub sstatus: usize,
    pub sepc: usize,
    pub stval: usize,
    pub scause: usize,
    pub kernel_satp: usize,
    pub kernel_sp: usize,
    pub kernel_trap: usize,
}

pub fn exceptions_init() -> Result<(), &'static str> {
    unsafe {
        unsafe extern "C" {
            fn __alltraps();
        }
        riscv::register::stvec::write(
            __alltraps as usize,
            riscv::register::stvec::TrapMode::Direct,
        );
    }
    Ok(())
}

pub fn secondary_exceptions_init() {
    let _ = exceptions_init();
}

#[unsafe(no_mangle)]
pub extern "C" fn trap_handler(tf: &mut TrapFrame) {
    let scause = scause::read();
    let stval = riscv::register::stval::read();

    match scause.cause() {
        Trap::Exception(e) if e == Exception::UserEnvCall as usize => {
            tf.sepc += 4;
        }
        Trap::Exception(e) if e == Exception::LoadPageFault as usize || e == Exception::StorePageFault as usize => {
            panic!("Page Fault at {:#x}, addr={:#x}", tf.sepc, stval);
        }
        Trap::Interrupt(i) if i == Interrupt::SupervisorTimer as usize => {
        }
        _ => {
            panic!(
                "Unhandled Trap: {:?} (code: {}) at {:#x}, stval={:#x}", 
                scause.cause(), 
                scause.code(),
                tf.sepc, 
                stval
            );
        }
    }
}