use core::arch::asm;
use libkernel::memory::address::PA;

// RISC-V SBI 常量定义 (Hart State Management - HSM Extension)
// 扩展 ID: 0x48534D ('HSM')
const SBI_EXT_HSM: usize = 0x48534D; 
// 功能 ID: 0x3 (HART_START)
const SBI_HSM_HART_START: usize = 0x3;
// SBI 返回值常量
const SBI_SUCCESS: isize = 0;
const SBI_ERR_ALREADY_AVAILABLE: isize = -6;

/// 执行一个带 3 个参数的通用 SBI 调用（RISC-V 64 位 ABI）。
///
/// 对应于：sbi_call_3(hart_id, start_addr, opaque)
/// 参数通过 a0, a1, a2 传递；功能 ID (FID) 在 a6，扩展 ID (EID) 在 a7。
/// 返回值遵循 SBI 规范：(错误码 error_code in a0, 返回值 return_value in a1)。
#[inline(always)]
fn sbi_hsm_hart_start_call(hart_id: usize, start_addr: usize, opaque: usize) -> (isize, usize) {
    let mut a0 = hart_id;
    let mut a1 = start_addr;
    let ret: usize;
    unsafe {
        // ecall 执行 SBI 调用。
        asm!(
            "ecall",
            inlateout("a0") a0 => ret, // a0 (ret) 获取错误码 (error code)
            inlateout("a1") a1,        // a1 (a1) 获取返回值 (return value)
            in("a2") opaque,           // a2 作为第三个参数
            in("a6") SBI_HSM_HART_START, // a6 = FID
            in("a7") SBI_EXT_HSM,        // a7 = EID
            options(nostack, nomem, preserves_flags) // 优化选项：无栈操作、无内存副作用、保留标志位
        );
    }
    
    // RISC-V SBI ABI: a0 is error_code (isize), a1 is return value (usize/u64).
    (ret as isize, a1)
}

/// 启动一个次级 Hart（CPU 核心），功能上等同于 ARM 的 PSCI CPU_ON 操作。
///
/// # 参数
/// * `hart_id`: 要启动的次级 Hart 的 ID。
/// * `entry_fn`: Hart 开始执行的物理地址（PA）。
/// * `ctx`: 传递给新启动 Hart 的上下文值（PA），它将在新 Hart 的 a1 寄存器中可读。
///
/// # 返回值
/// 成功启动（包括核心已在运行）返回 `Ok(())`，否则返回一个描述 SBI 错误的字符串。
pub fn boot_secondary_hart(hart_id: usize, entry_fn: PA, ctx: PA) -> Result<(), &'static str> {
    let (err, _) = sbi_hsm_hart_start_call(
        hart_id,
        entry_fn.value(), // start_addr in a1
        ctx.value(),      // opaque in a2 (将复制到新 Hart 的 a1 寄存器)
    );

    match err {
        SBI_SUCCESS => {
            // Hart 成功启动。
            Ok(())
        },
        SBI_ERR_ALREADY_AVAILABLE => {
            // Hart 已经处于可运行状态，这在启动流程中通常视为成功。
            Ok(())
        },
        // 详细错误码翻译（遵循 SBI 规范）
        _ => {
            match err {
                -1 => Err("SBI HART_START 失败: ERR_FAILED (操作失败)"),
                -2 => Err("SBI HART_START 失败: ERR_NOT_SUPPORTED (不支持 HSM 扩展或 HART_START 功能)"),
                -3 => Err("SBI HART_START 失败: ERR_INVALID_PARAM (参数无效，例如 hart_id 不存在)"),
                -4 => Err("SBI HART_START 失败: ERR_DENIED (权限不足)"),
                -5 => Err("SBI HART_START 失败: ERR_INVALID_ADDRESS (启动地址无效)"),
                _ => Err("SBI HART_START 失败: 未知错误码"),
            }
        }
    }
}