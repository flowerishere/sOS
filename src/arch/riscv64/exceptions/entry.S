.altmacro
.macro SAVE_GP n
    sd x\n, \n*8(sp)
.endm
.macro LOAD_GP n
    ld x\n, \n*8(sp)
.endm

    .section .text
    .align 2
    .global __alltraps
    .global exception_return
    .global __restore

# ----------------------------------
# Trap 入口: __alltraps
# ----------------------------------
__alltraps:
    # 交换 sp 和 sscratch
    # 情况 1 (来自用户态): sscratch = 内核栈顶, sp = 用户栈 -> 交换后 sp = 内核栈顶
    # 情况 2 (来自内核态): sscratch = 0, sp = 内核栈 -> 交换后 sp = 0
    csrrw sp, sscratch, sp
    
    # 检查 sp 是否为 0 (判断是否来自内核态)
    bnez sp, .Lfrom_user
    
    # === 来自内核态 (Kernel Trap) ===
    # 恢复 sp (将 sscratch 中的内核 sp 读回，sscratch 变回 0)
    csrr sp, sscratch
    j .Lsave_context

.Lfrom_user:
    # === 来自用户态 (User Trap) ===
    # 此时 sp 已经指向内核栈顶，sscratch 保存了用户态 sp
    # 继续向下执行保存流程

.Lsave_context:
    # 在栈上分配 TrapFrame 空间
    # 32 个通用寄存器 + 5 个 CSR (sstatus, sepc, stval, scause, sscratch/user_sp) = 37 * 8
    addi sp, sp, -37*8
    
    # 保存通用寄存器 x1, x3 ~ x31
    sd x1, 1*8(sp)
    sd x3, 3*8(sp)
    .set n, 4
    .rept 28
        SAVE_GP %n
        .set n, n+1
    .endr

    # 保存 CSR 状态
    csrr t0, sstatus
    csrr t1, sepc
    csrr t2, stval
    csrr t3, scause
    sd t0, 32*8(sp)
    sd t1, 33*8(sp)
    sd t2, 34*8(sp)
    sd t3, 35*8(sp)

    # 保存原来的 sp (x2)
    # sscratch 此时保存的是 Trap 发生前的 sp (用户栈或内核栈)
    csrr t0, sscratch
    sd t0, 2*8(sp)

    # 准备调用 Rust 函数: fn trap_handler(tf: &mut TrapFrame)
    # a0 参数指向当前栈顶 (TrapFrame 起始位置)
    mv a0, sp
    call trap_handler

# ----------------------------------
# Trap 出口: exception_return / __restore
# ----------------------------------
# void exception_return(TrapFrame *ctx);
# a0 参数指向 TrapFrame 结构体 (可能是同一个栈，也可能是切换后的新线程栈)
exception_return:
__restore:
    # 1. 切换栈指针到 TrapFrame
    # 如果发生了任务切换，a0 指向新任务的 TrapFrame，我们需要切换 sp
    mv sp, a0

    # 2. 恢复 CSR 状态
    ld t0, 32*8(sp)
    ld t1, 33*8(sp)
    ld t2, 34*8(sp)
    ld t3, 35*8(sp)
    csrw sstatus, t0
    csrw sepc, t1
    csrw stval, t2
    csrw scause, t3

    # 3. 恢复通用寄存器 x1, x3 ~ x31
    ld x1, 1*8(sp)
    ld x3, 3*8(sp)
    .set n, 4
    .rept 28
        LOAD_GP %n
        .set n, n+1
    .endr

    # 4. 恢复栈指针 (sp) 并返回
    # 我们需要根据 sstatus.SPP 判断是返回用户态还是内核态
    ld t0, 32*8(sp)      # 重新加载 sstatus
    andi t0, t0, 0x100   # 提取 SPP 位 (Bit 8)
    bnez t0, .Lreturn_to_kernel

    # === 返回用户态 ===
    # 逻辑: 
    # 1. sscratch 必须指向内核栈顶 (以便下次 Trap 使用)
    # 2. sp 必须恢复为用户栈指针
    
    # 计算内核栈顶位置 (当前 sp + TrapFrame 大小)
    addi t0, sp, 37*8
    csrw sscratch, t0    # 设置 sscratch = 内核栈顶
    
    ld sp, 2*8(sp)       # 恢复用户栈 sp
    sret

.Lreturn_to_kernel:
    # === 返回内核态 ===
    # 逻辑: 
    # 1. sscratch 必须保持为 0 (表示如果在内核态发生 Trap，递归处理)
    # 2. sp 恢复为内核栈指针
    
    ld sp, 2*8(sp)       # 恢复内核栈 sp
    sret