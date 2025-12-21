.altmacro
.macro SAVE_GP n
    sd x\n, \n*8(sp)
.endm
.macro LOAD_GP n
    ld x\n, \n*8(sp)
.endm

.align 2
.global __alltraps
__alltraps:
    # 交换 sp 和 sscratch。
    # 如果来自用户态：sscratch 保存的是内核栈，sp 是用户栈 -> 交换后 sp 指向内核栈。
    # 如果来自内核态：sscratch 为 0 -> 交换后 sp 为 0 (需要检查)。
    csrrw sp, sscratch, sp
    
    # 检查 sp 是否为 0 (来自内核态)
    bnez sp, .Lfrom_user
    # 如果 sp 为 0，说明我们已经在内核态，sscratch 也是 0。
    # 恢复 sp，并直接在当前栈上保存
    csrr sp, sscratch
    j .Lsave_context

.Lfrom_user:
    # 来自用户态，sp 现已指向 TrapFrame (内核栈底预留空间)
    # 继续

.Lsave_context:
    # 在栈上分配 TrapFrame 空间 (32regs + 5 CSRs = 37 * 8)
    addi sp, sp, -37*8
    
    # 保存通用寄存器
    sd x1, 1*8(sp)
    # x2 (sp) 需要特殊处理
    sd x3, 3*8(sp)
    .set n, 4
    .rept 28
        SAVE_GP %n
        .set n, n+1
    .endr

    # 保存 CSR
    csrr t0, sstatus
    csrr t1, sepc
    csrr t2, stval
    csrr t3, scause
    sd t0, 32*8(sp)
    sd t1, 33*8(sp)
    sd t2, 34*8(sp)
    sd t3, 35*8(sp)

    # 处理用户栈指针 (x2)
    # sscratch 现在保存的是用户原来的 sp (或者内核原来的 sp，如果是内核重入)
    csrr t0, sscratch
    sd t0, 2*8(sp)

    # 调用 Rust 处理函数
    mv a0, sp
    call trap_handler

    # 恢复上下文
    # ... (RESTORE 逻辑略，对称操作)
    # 最后 sret