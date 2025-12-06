.section .text.boot
.global _start

#
# RISC-V Kernel Entry Point
# a0 = Hart ID
# a1 = DTB Physical Address
#
_start:
    # Disable interrupts
    csrw sie, zero
    csrw sip, zero

    # Save boot arguments (Hart ID and DTB) to Saved Registers
    mv s0, a0
    mv s1, a1

    # If not primary core (Hart 0), park it.
    bnez a0, .Lsecondary_park

    # Clear BSS section
    la t0, __bss_start
    la t1, __bss_end
1:  bge t0, t1, 2f
    sd zero, 0(t0)
    addi t0, t0, 8
    j 1b

2:
    # Setup temporary boot stack (physical address)
    la sp, __boot_stack

    # Call Rust paging initialization
    # fn paging_bootstrap(static_pages: PA, image_start: PA, dtb: PA) -> SatpVal
    la a0, __init_pages_start # Arg 0: Static memory area for page tables
    la a1, __image_start      # Arg 1: Kernel image physical start address
    mv a2, s1                 # Arg 2: DTB physical address
    call paging_bootstrap

    # paging_bootstrap returns the value to be written to satp in a0

    # Enable MMU and jump to high address
    # Write to satp
    csrw satp, a0
    sfence.vma

    # Calculate jump target in high memory
    # load the symbol's address which the linker has placed in high memory
    la ra, .Lhigh_mem
    
    # Calculate the offset between virtual and physical address to adjust SP
    # Assumption: linker script places kernel at 0xffff8000...
    li t0, 0xffff800000000000  # Kernel Base VA (Must match linker.ld)
    la t1, __image_start       # Kernel Base PA
    sub t0, t0, t1             # Offset = VA - PA
    
    # Update stack pointer to virtual address
    add sp, sp, t0
    
    # Jump to high memory
# 'ra' already contains the high address because 'la' resolves to the
    # linked address (virtual), but we are currently running physically.
    # However, since we just enabled MMU with identity mapping, 
    # we can technically just jump. But to be safe and "switch" to VA execution:
    jr ra

.Lhigh_mem:
    # Setup stack frame and enter Rust world (Stage 1)
    # fn arch_init_stage1(dtb: TPA<u8>, image_start: PA, image_end: PA, satp: usize) -> VA
    mv a0, s1                 # Arg 0: DTB
    la a1, __image_start      # Arg 1: image_start
    la a2, __image_end        # Arg 2: image_end
    csrr a3, satp             # Arg 3: satp (pagetable base)
    
    call arch_init_stage1

    # Stage 1 returns the new kernel stack top (VA)
    mv sp, a0

    # Enter Stage 2 (kmain)
    # Reserve Context Switch Frame (TrapFrame)
    # 34 registers (x0-x31 + sstatus + sepc) * 8 bytes = 272 bytes
    addi sp, sp, -272
    mv a0, sp                 # Arg 0: Frame pointer
    call arch_init_stage2

    # Should not return
    j .

.Lsecondary_park:
    wfi
    j .Lsecondary_park