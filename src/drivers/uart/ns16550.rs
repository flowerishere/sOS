use crate::{
    arch::ArchImpl,
    drivers::{
        DeviceDescriptor, DriverManager,
        init::PlatformBus,
        probe::{DeviceMatchType, FdtFlags},
        uart::{UART_CHAR_DEV, Uart, UartDriver},
    },
};
use alloc::{boxed::Box, sync::Arc};
use core::{fmt, hint::spin_loop};
use libkernel::{
    KernAddressSpace, VirtualMemory,
    error::{ProbeError, Result},
    memory::{
        address::{PA, VA},
        region::PhysMemoryRegion,
    },
};
use tock_registers::{
    register_bitfields, register_structs,
    interfaces::{Readable, Writeable, ReadWriteable},
    registers::{ReadOnly, ReadWrite},
};

// NS16550A 寄存器位定义
register_bitfields! [
    u8,
    /// Interrupt Enable Register
    IER [
        ERBFI OFFSET(0) NUMBITS(1) [] // Enable Received Data Available Interrupt
    ],
    /// FIFO Control Register
    FCR [
        ENABLE OFFSET(0) NUMBITS(1) []
    ],
    /// Line Control Register
    LCR [
        WLS OFFSET(0) NUMBITS(2) [
            FiveBit = 0,
            SixBit = 1,
            SevenBit = 2,
            EightBit = 3
        ],
        DLAB OFFSET(7) NUMBITS(1) [] // Divisor Latch Access Bit
    ],
    /// Line Status Register
    LSR [
        DR OFFSET(0) NUMBITS(1) [],   // Data Ready
        THRE OFFSET(5) NUMBITS(1) []  // Transmitter Holding Register Empty
    ]
];

// NS16550A 寄存器布局
register_structs! {
    #[allow(non_snake_case)]
    pub Ns16550Regs {
        // Offset 0x00: RBR (Read) / THR (Write) / DLL (DLAB=1)
        (0x00 => pub rbr_thr_dll: ReadWrite<u8>),
        // Offset 0x01: IER / DLM (DLAB=1)
        (0x01 => pub ier_dlm: ReadWrite<u8, IER::Register>),
        // Offset 0x02: IIR (Read) / FCR (Write)
        (0x02 => pub iir_fcr: ReadWrite<u8, FCR::Register>),
        // Offset 0x03: LCR
        (0x03 => pub lcr: ReadWrite<u8, LCR::Register>),
        // Offset 0x04: MCR
        (0x04 => pub mcr: ReadWrite<u8>),
        // Offset 0x05: LSR
        (0x05 => pub lsr: ReadOnly<u8, LSR::Register>),
        // Offset 0x06: MSR
        (0x06 => pub msr: ReadOnly<u8>),
        // Offset 0x07: SCR
        (0x07 => pub scr: ReadWrite<u8>),
        (0x08 => @END),
    }
}

pub struct Ns16550 {
    regs: &'static Ns16550Regs,
}

// 驱动需要跨线程共享
unsafe impl Send for Ns16550 {}
unsafe impl Sync for Ns16550 {}

impl Ns16550 {
    /// 创建并初始化 UART 实例
    ///
    /// # Safety
    /// `base` 必须是映射到 NS16550 物理地址的有效虚拟地址
    pub unsafe fn new(base: VA) -> Self {
        // 将 VA 转换为寄存器结构体引用
        // 安全说明：调用者保证 base 有效且已映射
        let ptr = base.as_ptr() as *const Ns16550Regs;
        let regs = unsafe { &*ptr };
        
        let mut dev = Self { regs };
        dev.init();
        dev
    }

    fn init(&mut self) {
        // 1. 关闭中断
        self.regs.ier_dlm.set(0x00);

        // 2. 设置波特率 (QEMU 忽略此步骤，但在真实硬件上是必须的)
        self.regs.lcr.write(LCR::DLAB::SET);
        self.regs.rbr_thr_dll.set(0x03); // Divisor Latch Low (示例值)
        self.regs.ier_dlm.set(0x00);     // Divisor Latch High
        
        // 3. 设置数据格式: 8 bits, no parity, 1 stop bit (8N1), 并清除 DLAB
        self.regs.lcr.write(LCR::DLAB::CLEAR + LCR::WLS::EightBit);

        // 4. 启用 FIFO
        self.regs.iir_fcr.write(FCR::ENABLE::SET);

        // 5. 启用接收中断
        self.regs.ier_dlm.write(IER::ERBFI::SET);
    }

    // 内部辅助方法：单字节发送
    fn putc(&self, c: u8) {
        // 等待发送保持寄存器为空 (THRE)
        while !self.regs.lsr.is_set(LSR::THRE) {
            spin_loop();
        }
        self.regs.rbr_thr_dll.set(c);
    }

    // 内部辅助方法：单字节接收
    fn getc(&self) -> Option<u8> {
        // 检查数据是否就绪 (DR)
        if self.regs.lsr.is_set(LSR::DR) {
            Some(self.regs.rbr_thr_dll.get())
        } else {
            None
        }
    }
}

// 必须实现 fmt::Write 才能作为 Console 使用
impl fmt::Write for Ns16550 {
    fn write_str(&mut self, s: &str) -> fmt::Result {
        for c in s.bytes() {
            self.putc(c);
        }
        Ok(())
    }
}

// 适配 sOS 的 UartDriver Trait
impl UartDriver for Ns16550 {
    fn write_buf(&mut self, buf: &[u8]) {
        for &byte in buf {
            self.putc(byte);
        }
    }

    /// 从硬件 FIFO 读取所有可用字节到 buffer 中
    /// 返回读取的字节数
    fn drain_uart_rx(&mut self, buf: &mut [u8]) -> usize {
        let mut count = 0;
        for dest in buf.iter_mut() {
            if let Some(c) = self.getc() {
                *dest = c;
                count += 1;
            } else {
                break;
            }
        }
        count
    }
}

/// 注册 NS16550 驱动到 PlatformBus
pub fn ns16550_init(bus: &mut PlatformBus, _dm: &mut DriverManager) -> Result<()> {
    bus.register_platform_driver(
        // QEMU RISC-V virt 机器通常使用 "ns16550a"
        DeviceMatchType::FdtCompatible("ns16550a"),
        Box::new(|dm, desc| {
            // 修正：从 DeviceDescriptor::Fdt 中同时解包出 node 和 flags
            // DeviceDescriptor::Fdt 包含 (fdt_parser::Node, FdtFlags)
            let (fdt_node, flags) = match desc {
                DeviceDescriptor::Fdt(node, flags) => (node, flags),
                // 如果不是 FDT 设备，返回错误
                _ => return Err(libkernel::error::KernelError::Probe(ProbeError::NoReg)), 
            };
            
            // 1. 获取寄存器基地址和大小
            let mut regs = fdt_node.reg().ok_or(ProbeError::NoReg)?;
            let region = regs.next().ok_or(ProbeError::NoReg)?;
            let size = region.size.ok_or(ProbeError::NoRegSize)?;

            // 2. 映射 MMIO 内存
            // PA::from_value 不需要泛型参数，它会推断或使用默认
            let mem = ArchImpl::kern_address_space()
                .lock_save_irq()
                .map_mmio(PhysMemoryRegion::new(
                    PA::from_value(region.address as usize),
                    size,
                ))?;

            // 3. 解析并申请中断
            let mut interrupts = fdt_node
                .interrupts()
                .ok_or(ProbeError::NoInterrupts)?
                .next()
                .ok_or(ProbeError::NoInterrupts)?;

            let interrupt_node = fdt_node
                .interrupt_parent()
                .ok_or(ProbeError::NoParentIntterupt)?
                .node;

            let interrupt_manager = dm
                .find_by_name(interrupt_node.name)
                .ok_or(ProbeError::Deferred)?
                .as_interrupt_manager()
                .ok_or(ProbeError::NotInterruptController)?;

            let interrupt_config = interrupt_manager.parse_fdt_interrupt_regs(&mut interrupts)?;

            // 4. 创建驱动实例并注册中断处理函数
            let dev = interrupt_manager.claim_interrupt(interrupt_config, |claimed_interrupt| {
                unsafe {
                    Uart::new(Ns16550::new(mem), claimed_interrupt, fdt_node.name)
                }
            })?;

            // 5. 如果是活跃控制台，注册到字符设备层
            let uart_cdev = UART_CHAR_DEV.get().ok_or(ProbeError::Deferred)?;
            uart_cdev.register_console(dev.clone(), flags.contains(FdtFlags::ACTIVE_CONSOLE))?;

            Ok(dev)
        }),
    );

    Ok(())
}
crate::kernel_driver!(ns16550_init);