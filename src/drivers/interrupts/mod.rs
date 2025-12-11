#[cfg(any(feature = "arch-aarch64", target_arch = "aarch64"))]
pub mod arm_gic_v2;
#[cfg(any(feature = "arch-aarch64", target_arch = "aarch64"))]
pub mod arm_gic_v3;