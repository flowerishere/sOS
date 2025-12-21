use crate::CpuOps;
use super::Riscv64;
use core::arch::asm;

// sstatus 寄存器中 Supervisor Interrupt Enable (SIE) 是第 1 位
const SSTATUS_SIE_MASK: usize = 0x2;

impl CpuOps for Riscv64 {
    /// 获取当前 CPU 核的 ID (Hart ID)
    /// 假设启动代码(start.s)已经将 hartid 放入 tp 寄存器，且内核运行期间 tp 保持不变
    #[inline(always)]
    fn id() -> usize {
        let hartid: usize;
        unsafe {
            // 直接读取 tp 寄存器
            asm!("mv {0}, tp", out(reg) hartid);
        }
        hartid
    }

    /// 停机等待中断 (Wait For Interrupt)
    fn halt() -> ! {
        loop {
            // wfi 指令让 CPU 进入低功耗等待状态，直到下一个中断到来
            unsafe { asm!("wfi") };
        }
    }

    /// 开启中断
    #[inline(always)]
    fn enable_interrupts() {
        unsafe {
            // csrrs: Atomic Read and Set Bits
            // 将 sstatus 的 SIE 位置 1
            asm!("csrrs x0, sstatus, {0}", in(reg) SSTATUS_SIE_MASK);
        }
    }

    /// 关闭中断并返回之前的状态
    #[inline(always)]
    fn disable_interrupts() -> usize {
        let flags: usize;
        unsafe {
            // csrrc: Atomic Read and Clear Bits
            // 1. 读取 sstatus 的旧值到 flags
            // 2. 将 sstatus 的 SIE 位清 0
            // 这是一个原子操作，比 "read -> modify -> write" 更安全高效
            asm!("csrrc {0}, sstatus, {1}", out(reg) flags, in(reg) SSTATUS_SIE_MASK);
        }
        flags
    }

    /// 恢复之前保存的中断状态
    #[inline(always)]
    fn restore_interrupt_state(flags: usize) {
        unsafe {
            // 必须处理两种情况：
            // 1. 如果保存的状态是开启 (Bit 1 == 1)，则开启
            // 2. 如果保存的状态是关闭 (Bit 1 == 0)，则关闭 (这一步在你之前的代码中缺失)
            if (flags & SSTATUS_SIE_MASK) != 0 {
                asm!("csrrs x0, sstatus, {0}", in(reg) SSTATUS_SIE_MASK);
            } else {
                asm!("csrrc x0, sstatus, {0}", in(reg) SSTATUS_SIE_MASK);
            }
        }
    }
}