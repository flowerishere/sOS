use core::arch::asm;
use libkernel::memory::address::PA;

// ============================================================================
// RISC-V SBI Hart State Management (HSM) 常量
// 规范文档: RISC-V Supervisor Binary Interface Specification, Chapter 8
// ============================================================================

// Extension ID: 'HSM' (Hart State Management)
const SBI_EXT_HSM: usize = 0x48534D;

// Function IDs
const SBI_FID_HART_START: usize = 0;
#[allow(dead_code)]
const SBI_FID_HART_STOP: usize = 1;
#[allow(dead_code)]
const SBI_FID_HART_GET_STATUS: usize = 2;
#[allow(dead_code)]
const SBI_FID_HART_SUSPEND: usize = 3;

// SBI Return Codes
const SBI_SUCCESS: isize = 0;
const SBI_ERR_ALREADY_AVAILABLE: isize = -6;
const SBI_ERR_ALREADY_STARTED: isize = -7;

/// 启动一个次级 Hart（CPU 核心）。
///
/// 这是 `arch::arm64::boot_secondary_psci` 的 RISC-V 对等实现。
/// 它调用 SBI HSM 扩展的 `sbi_hart_start`。
///
/// # 参数
/// * `hart_id`: 目标 Hart 的物理 ID (从 Device Tree 或 ACPI 获取)。
/// * `entry_fn`: 目标 Hart 启动后跳转的物理地址 (通常是 `secondary_start` 汇编标签)。
/// * `ctx`: 传递给目标 Hart 的 `a1` 寄存器的不透明值 (通常是 `Cpu` 结构体指针或页表地址)。
pub fn boot_secondary_hart(hart_id: usize, entry_fn: PA, ctx: PA) -> Result<(), &'static str> {
    // 调用 sbi_hart_start(hartid, start_addr, opaque)
    let (error, _value) = unsafe {
        sbi_call_3(
            SBI_EXT_HSM,
            SBI_FID_HART_START,
            hart_id,
            entry_fn.value(),
            ctx.value(),
        )
    };

    match error {
        SBI_SUCCESS => Ok(()),
        // 如果核心已经在运行或已准备好，我们通常认为这是成功的（幂等性）
        SBI_ERR_ALREADY_AVAILABLE | SBI_ERR_ALREADY_STARTED => Ok(()),
        
        // 详细错误映射，便于调试 panic
        -1 => Err("SBI: Failed (ERR_FAILED)"),
        -2 => Err("SBI: HSM Not Supported (ERR_NOT_SUPPORTED)"),
        -3 => Err("SBI: Invalid Param (ERR_INVALID_PARAM) - Check Hart ID"),
        -4 => Err("SBI: Denied (ERR_DENIED)"),
        -5 => Err("SBI: Invalid Address (ERR_INVALID_ADDRESS) - Check entry_fn alignment/validity"),
        _ => Err("SBI: Unknown Error"),
    }
}

/// 原始 SBI 调用封装 (3个参数)
///
/// 遵循 RISC-V SBI 调用约定：
/// - Args: a0, a1, a2, a3, a4, a5
/// - FID: a6
/// - EID: a7
/// - Return: a0 (error), a1 (value)
#[inline(always)]
unsafe fn sbi_call_3(eid: usize, fid: usize, arg0: usize, arg1: usize, arg2: usize) -> (isize, usize) {
    let error: isize;
    let value: usize;
    asm!(
        "ecall",
        in("a7") eid,
        in("a6") fid,
        inlateout("a0") arg0 => error,
        inlateout("a1") arg1 => value,
        in("a2") arg2,
        options(nostack, preserves_flags)
    );
    (error, value)
}