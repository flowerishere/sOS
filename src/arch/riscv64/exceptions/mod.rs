// src/arch/riscv64/exceptions/mod.rs

use core::arch::global_asm;
use riscv::register::scause::{self, Trap, Exception, Interrupt}; 
// 注意：riscv 0.12 中 Exception/Interrupt 通常在 riscv::interrupt 中，
// 但 scause::read().cause() 返回的 Trap 枚举包裹了它们。
// 如果编译仍报错，改为：use riscv::interrupt::{Exception, Interrupt};

global_asm!(include_str!("entry.S")); // 确保文件名大小写一致，之前是 entry.S

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

#[unsafe(no_mangle)] // 修复：使用 unsafe(no_mangle) 适配新版 Rust
pub extern "C" fn trap_handler(tf: &mut TrapFrame) {
    let scause = scause::read();
    let stval = riscv::register::stval::read();

    match scause.cause() {
        Trap::Exception(Exception::UserEnvCall) => {
            tf.sepc += 4;
        }
        Trap::Exception(Exception::LoadPageFault) |
        Trap::Exception(Exception::StorePageFault) => {
            panic!("Page Fault at {:#x}, addr={:#x}", tf.sepc, stval);
        }
        Trap::Interrupt(Interrupt::SupervisorTimer) => {
            // Timer logic
        }
        _ => {
            panic!("Unhandled Trap: {:?} at {:#x}, stval={:#x}", scause.cause(), tf.sepc, stval);
        }
    }
}