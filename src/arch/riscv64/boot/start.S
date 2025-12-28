.section .text.boot
.global _start

_start:
    la t0, .Learly_trap_handler
    csrw stvec, t0
    
    # SBI Console Putchar: a7=1, a6=0, a0=char
    li a7, 1              # SBI_CONSOLE_PUTCHAR
    li a6, 0
    li a0, 65             # 'A'
    ecall
    
    csrw sie, zero
    csrw sip, zero

    mv s0, a0
    mv s1, a1

    # === Debug 2 ===
    li a7, 1
    li a6, 0
    li a0, 66
    ecall

    la t0, _start
    li t1, 0x80200000
    sub s3, t0, t1

    # === Debug 3 ===
    li a7, 1
    li a6, 0
    li a0, 67             # 'C'
    ecall

    la sp, __boot_stack
    sub sp, sp, s3

    # === Debug 4 ===
    li a7, 1
    li a6, 0
    li a0, 68             # 'D'
    ecall

    la t0, __init_pages_start
    sub a0, t0, s3

    la t0, __image_start
    sub a1, t0, s3

    mv a2, s1


    call paging_bootstrap


    
    mv s2, a0
    
    la ra, .Lhigh_mem
    jr ra

.Lhigh_mem:

    
    li a7, 1
    li a6, 0
    li a0, 69             # 'E'
    ecall

    la sp, __boot_stack

    # === Debug ===
    li a7, 1
    li a6, 0
    li a0, 70             # 'F'
    ecall

    mv a0, s1

    la t0, __image_start
    sub a1, t0, s3

    la t0, __image_end
    sub a2, t0, s3

    mv a3, s2

    call arch_init_stage1

    li a7, 1
    li a6, 0
    li a0, 71             # 'G'
    ecall

    mv sp, a0

    addi sp, sp, -272
    mv a0, sp

    call arch_init_stage2

.Lhang:
    wfi
    j .Lhang

.align 4
.Learly_trap_handler:

    li a7, 1
    li a6, 0
    li a0, 88             # 'X'
    ecall

    csrr t0, scause
    csrr t1, sepc
    csrr t2, stval

.Ltrap_loop:
    wfi
    j .Ltrap_loop

.Lsecondary_park:
    li a7, 1
    li a6, 0
    li a0, 90             # 'Z'
    ecall

    wfi
    j .Lsecondary_park