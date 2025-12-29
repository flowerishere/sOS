use crate::memory::PageOffsetTranslator;
use libkernel::{
    arch::riscv64::memory::pg_tables::{PageAllocator, PgTable, PgTableArray},
    error::{Result, KernelError}, // 引入 KernelError
    memory::{PAGE_SIZE, address::TPA, smalloc::Smalloc},
};

// 简单的调试打印工具 (因为这里还没法用 println!)
unsafe fn debug_uart_putc(c: u8) {
    let ptr = 0x1000_0000 as *mut u8;
    core::ptr::write_volatile(ptr, c);
}

unsafe fn debug_puts(s: &str) {
    for c in s.bytes() { debug_uart_putc(c); }
}

unsafe fn debug_print_hex(mut val: usize) {
    let hex_chars = b"0123456789abcdef";
    let mut buf = [0u8; 16];
    let mut i = 0;
    if val == 0 { debug_uart_putc(b'0'); return; }
    while val > 0 {
        buf[i] = hex_chars[val & 0xf];
        val >>= 4;
        i += 1;
    }
    while i > 0 { i -= 1; debug_uart_putc(buf[i]); }
}

pub struct SmallocPageAlloc<'a> {
    smalloc: &'a mut Smalloc<PageOffsetTranslator>,
}

impl<'a> SmallocPageAlloc<'a> {
    pub fn new(smalloc: &'a mut Smalloc<PageOffsetTranslator>) -> Self {
        Self { smalloc }
    }
}

impl PageAllocator for SmallocPageAlloc<'_> {
    fn allocate_page_table<T: PgTable>(&mut self) -> Result<TPA<PgTableArray<T>>> {
        // 1. 尝试分配
        let alloc_res = self.smalloc.alloc(PAGE_SIZE, PAGE_SIZE);
        
        // 2. 错误处理与调试
        match alloc_res {
            Ok(addr) => {
                let pa_val = addr.value();
                
                // [CRITICAL CHECK] 检查是否分配了 0 地址
                if pa_val == 0 {
                    unsafe {
                        debug_puts("\n[FATAL] SmallocPageAlloc returned physical address 0!\n");
                        debug_puts("This causes Store Access Fault when Fixmap tries to map it.\n");
                        debug_puts("Check your kmain allocator initialization!\n");
                    }
                    // 死循环以保留现场
                    loop {}
                }

                // 可选：打印分配到的地址，看看是否合理（应该很大，比如 > 0x80000000）
                /*
                unsafe {
                    debug_puts("[ALLOC] PA: 0x");
                    debug_print_hex(pa_val);
                    debug_puts("\n");
                }
                */

                Ok(TPA::from_value(pa_val))
            }
            Err(e) => {
                unsafe {
                    debug_puts("\n[ERROR] SmallocPageAlloc failed to allocate memory!\n");
                    // 打印错误码 (如果 e 能转成数字的话)
                }
                Err(e)
            }
        }
    }
}