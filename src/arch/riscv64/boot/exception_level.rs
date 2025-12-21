use core::arch::asm;

/// 切换到内核的主栈并禁用中断。
///
/// # 警告
/// 这是一个 `#[naked]` 函数。它不使用标准的函数调用约定（不保存寄存器，不创建栈帧）。
/// 它直接修改 `sp` 寄存器，因此必须由汇编代码小心控制。
///
/// 参数 `stack_addr` 根据 RISC-V C ABI 位于 `a0` 寄存器中。
#[naked]
#[no_mangle]
pub unsafe extern "C" fn transition_to_sv_mode(stack_addr: u64) {
    asm!(
        // 1. 禁用中断 (Clear SIE)
        // sstatus.SIE 是第 1 位 (mask 0x2)
        // 我们使用 CSR 指令直接操作，避免依赖外部函数调用的栈开销
        "li t0, 0x2",
        "csrc sstatus, t0",

        // 2. 设置栈指针
        // 参数 stack_addr 已经在 a0 寄存器中
        "mv sp, a0",

        // 3. 返回调用者
        // ret 伪指令等同于 jalr x0, 0(ra)
        // 此时 sp 已经切换，但 ra 仍然是调用者设置的（未被破坏），所以可以安全返回
        "ret",
        options(noreturn)
    );
}