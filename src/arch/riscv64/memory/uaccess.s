// libkernel/src/arch/riscv64/memory/uaccess.s

.section .text
.balign 4

// 标记 uaccess 代码段的开始，用于异常处理判断
.global __uaccess_begin
__uaccess_begin:

// -----------------------------------------------------------------------------
// __do_copy_from_user
// 从用户空间复制数据
//
// Arguments:
//   a0: src pointer (用户态地址)
//   a1: dst pointer (内核态地址)
//   a2: current offset (当前已复制字节数)
//   a3: total size (总长度)
//
// Returns:
//   a0: status (0 = success, 1 = denied, 2 = deferred)
//   a1: work pointer (future, if deferred)
//   a2: current offset (bytes copied)
// -----------------------------------------------------------------------------
.global __do_copy_from_user
.type __do_copy_from_user, @function
__do_copy_from_user:
    beq     a2, a3, 1f      // if offset == total_size, goto done
    
    add     t0, a0, a2      // t0 = src + offset
    lb      t1, 0(t0)       // load byte from user (可能触发异常)
    
    add     t2, a1, a2      // t2 = dst + offset
    sb      t1, 0(t2)       // store byte to kernel
    
    addi    a2, a2, 1       // offset++
    j       __do_copy_from_user

// -----------------------------------------------------------------------------
// __do_copy_from_user_halt_nul
// 从用户空间复制字符串，遇 \0 停止
// -----------------------------------------------------------------------------
.global __do_copy_from_user_halt_nul
.type __do_copy_from_user_halt_nul, @function
__do_copy_from_user_halt_nul:
    beq     a2, a3, 1f      // if offset == max_len, goto done
    
    add     t0, a0, a2      // t0 = src + offset
    lb      t1, 0(t0)       // load byte (可能触发异常)
    
    add     t2, a1, a2      // t2 = dst + offset
    sb      t1, 0(t2)       // store byte
    
    beqz    t1, 1f          // if byte == 0, goto done
    
    addi    a2, a2, 1       // offset++
    j       __do_copy_from_user_halt_nul

// -----------------------------------------------------------------------------
// __do_copy_to_user
// 复制数据到用户空间
// -----------------------------------------------------------------------------
.global __do_copy_to_user
.type __do_copy_to_user, @function
__do_copy_to_user:
    beq     a2, a3, 1f      // if offset == total_size, goto done
    
    add     t0, a0, a2      // t0 = src + offset
    lb      t1, 0(t0)       // load byte from kernel
    
    add     t2, a1, a2      // t2 = dst + offset
    sb      t1, 0(t2)       // store byte to user (可能触发异常)
    
    addi    a2, a2, 1       // offset++
    j       __do_copy_to_user

1:
    li      a0, 0           // Status = Success
    // a1 (work_ptr), a3 (work_vtable) 未定义，Rust侧不应读取
    ret

// 标记 uaccess 代码段结束
.global __uaccess_end
__uaccess_end:

// -----------------------------------------------------------------------------
// Fixup Handler
// 当 uaccess 发生异常且被内核处理后，Trap Handler 会将 SEPC 指向这里
// 此时 a0 已由 Rust 代码设置为错误状态 (Denied 或 Deferred)
// -----------------------------------------------------------------------------
.global fixup
fixup:
    ret

// -----------------------------------------------------------------------------
// Exception Fixup Table
// 定义内存布局，供 Rust 的 handle_kernel_mem_fault 读取
// -----------------------------------------------------------------------------
.section .exception_fixups, "a"
.global __UACCESS_FIXUP
.balign 8
__UACCESS_FIXUP:
    .quad   __uaccess_begin
    .quad   __uaccess_end
    .quad   fixup