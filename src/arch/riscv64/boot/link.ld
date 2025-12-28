OUTPUT_ARCH(riscv)
ENTRY(_start)

/* 定义内核虚拟基地址 (高地址) 和物理加载地址 */
BASE_ADDRESS = 0xffff800000000000;
LOAD_ADDRESS = 0x80200000;

/* 计算虚拟地址和物理地址的偏移量 */
KERNEL_OFFSET = BASE_ADDRESS - LOAD_ADDRESS;

SECTIONS
{
    /* 设置当前的虚拟地址计数器 */
    . = BASE_ADDRESS;

    __image_start = .;

    /* * .text 段
     * VMA (Virtual Memory Address) = 0xffff800000000000
     * LMA (Load Memory Address)    = 0x80200000
     * 使用 AT() 指定 LMA，这样加载器知道把代码放在物理内存哪里，
     * 而代码中的符号解析则使用 VMA。
     */
    .text : AT(ADDR(.text) - KERNEL_OFFSET) {
        __text_start = .;
        
        /* 确保启动代码在最前面 */
        KEEP(*(.text.boot))
        
        *(.text .text.*)
        . = ALIGN(4096);
        __text_end = .;
    }

    .rodata : AT(ADDR(.rodata) - KERNEL_OFFSET) {
        __rodata_start = .;
        *(.rodata .rodata.*)
        
        /* 驱动初始化函数列表 (用于 kernel_driver! 宏) */
        . = ALIGN(8);
        __driver_inits_start = .;
        KEEP(*(.driver_inits))
        __driver_inits_end = .;
        
        /* 异常修复表 (用于处理 copy_from_user 等产生的异常) */
        . = ALIGN(8);
        __fixups_start = .;
        *(.exception_fixups)
        __fixups_end = .;

        . = ALIGN(4096);
        __rodata_end = .;
    }

    .data : AT(ADDR(.data) - KERNEL_OFFSET) {
        __data_start = .;
        *(.data .data.*)
        *(.sdata .sdata.*)
        . = ALIGN(4096);
        __data_end = .;
    }

    /* Per-CPU 数据段 */
    .percpu : AT(ADDR(.percpu) - KERNEL_OFFSET) {
        . = ALIGN(8);
        __percpu_start = .;
        KEEP(*(.percpu))
        __percpu_end = .;
        . = ALIGN(4096);
    }

    .bss : AT(ADDR(.bss) - KERNEL_OFFSET) {
        __bss_start = .;
        *(.bss .bss.*)
        *(.sbss .sbss.*)
        . = ALIGN(4096);
        __bss_end = .;
    }

    /* 预留初始页表空间 (128个页面) */
    . = ALIGN(4096);
    __init_pages_start = .;
    . += 4096 * 128; 
    __init_pages_end = .;

    /* 预留启动栈空间 (16个页面) */
    . = ALIGN(4096);
    . += 4096 * 16;
    __boot_stack = .;
    
    __image_end = .;

    /* 丢弃不需要的段 */
    /DISCARD/ : {
        *(.eh_frame)
        *(.comment)
        *(.note.gnu.build-id)
    }
}