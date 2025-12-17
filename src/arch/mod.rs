#[cfg(any(target_arch = "aarch64", doc))]
mod arm64;

#[cfg(target_arch = "aarch64")]
pub use self::arm64::Aarch64 as ArchImpl;


#[cfg(any(target_arch = "riscv64", doc))]
mod riscv64;

#[cfg(target_arch = "riscv64")]
pub use self::riscv64::Riscv64 as ArchImpl;