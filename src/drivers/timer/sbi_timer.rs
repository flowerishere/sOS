// src/drivers/timer/sbi_timer.rs

use crate::drivers::{Driver, timer::{HwTimer, Instant}};
use core::arch::asm;

pub struct SbiTimer;

impl SbiTimer {
    pub const fn new() -> Self {
        Self
    }
}

impl Driver for SbiTimer {
    fn name(&self) -> &'static str {
        "sbi-timer"
    }
}

impl HwTimer for SbiTimer {
    fn now(&self) -> Instant {
        let time: u64;
        unsafe {
            asm!("csrr {}, time", out(reg) time);
        }
        // 假设频率 10MHz
        Instant { ticks: time, freq: 10_000_000 } 
    }

    fn schedule_interrupt(&self, when: Option<Instant>) {
        match when {
            Some(inst) => {
                sbi_rt::set_timer(inst.ticks);
                unsafe {
                    asm!("csrs sie, {}", in(reg) 0x20usize);
                }
            },
            None => {
                sbi_rt::set_timer(u64::MAX);
                unsafe {
                    asm!("csrc sie, {}", in(reg) 0x20usize);
                }
            }
        }
    }
}