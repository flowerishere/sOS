use crate::{
    arch::{ArchImpl, riscv64::TrapFrame},
    memory::{PageOffsetTranslator, page::ClaimedPage},
    process::Task,
};
use core::arch::global_asm;
use libkernel::{
    UserAddressSpace, VirtualMemory,
    memory::{
        address::VA,
        permissions::PtePermissions,
        proc_vm::vmarea::{VMAPermissions, VMArea, VMAreaKind},
        region::VirtMemoryRegion,
    },
};

global_asm!(include_str!("idle.s"));

pub fn create_idle_task() -> Task {
    // 分配一个物理页作为代码页
    let code_page = ClaimedPage::alloc_zeroed().unwrap().leak();
    // 任意选取一个用户态地址作为 Idle 任务的代码段地址
    let code_addr = VA::from_value(0xd00d0000);

    unsafe extern "C" {
        static __idle_start: u8;
        static __idle_end: u8;
    }

    let idle_start_ptr = unsafe { &__idle_start } as *const u8;
    let idle_end_ptr = unsafe { &__idle_end } as *const u8;
    let code_sz = idle_end_ptr.addr() - idle_start_ptr.addr();

    // 将汇编代码拷贝到分配的物理页中
    unsafe {
        idle_start_ptr.copy_to(
            code_page
                .pa()
                .to_va::<PageOffsetTranslator>()
                .cast::<u8>()
                .as_ptr_mut(),
            code_sz,
        )
    };

    // 创建新的用户地址空间
    let mut addr_space = <ArchImpl as VirtualMemory>::ProcessAddressSpace::new().unwrap();

    // 映射代码页，设置为可读可执行 (User mode)
    addr_space
        .map_page(code_page, code_addr, PtePermissions::rx(true))
        .unwrap();

    // 初始化 TrapFrame (RISC-V 的 ExceptionState)
    let mut ctx = TrapFrame {
        regs: [0; 32],
        sstatus: 0,
        sepc: code_addr.value(),
    };
    
    // 配置 sstatus:
    // SPIE (bit 5) = 1: 开启中断
    // SPP (bit 8) = 0: 异常发生前的特权级为 User
    ctx.sstatus = 1 << 5; 
    // SP (regs[2]) = 0; // Idle 任务不需要栈

    let code_map = VMArea::new(
        VirtMemoryRegion::new(code_addr, code_sz),
        VMAreaKind::Anon,
        VMAPermissions::rx(),
    );

    Task::create_idle_task(addr_space, ctx, code_map)
}