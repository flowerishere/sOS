OUTPUT_ARCH(riscv)
ENTRY(_start)

BASE_ADDRESS = 0xffff800000000000;
LOAD_ADDRESS = 0x80200000;

KERNEL_OFFSET = BASE_ADDRESS - LOAD_ADDRESS;

SECTIONS
{
    . = BASE_ADDRESS;

    __image_start = .;

    /* * .text
     * VMA = 0xffff800000000000
     * LMA = 0x80200000
     */
    .text : AT(ADDR(.text) - KERNEL_OFFSET) {
        __text_start = .;
        
        KEEP(*(.text.boot))
        
        *(.text .text.*)
        . = ALIGN(4096);
        __text_end = .;
    }

    .rodata : AT(ADDR(.rodata) - KERNEL_OFFSET) {
        __rodata_start = .;
        *(.rodata .rodata.*)
        
        . = ALIGN(8);
        __driver_inits_start = .;
        KEEP(*(.driver_inits))
        __driver_inits_end = .;
        
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

    . = ALIGN(4096);
    __init_pages_start = .;
    . += 4096 * 128; 
    __init_pages_end = .;

    . = ALIGN(4096);
    . += 4096 * 16;
    __boot_stack = .;
    
    __image_end = .;

    /DISCARD/ : {
        *(.eh_frame)
        *(.comment)
        *(.note.gnu.build-id)
    }
}