//! The processor mode VPX's physics runs in.
//!
//! VPX's player switches on flush-to-zero and denormals-are-zero (MXCSR bits
//! `0x8040`) on the thread that runs physics (`Player::Player`): numbers too
//! small for a float's normal range act as zero. The port must run in the same
//! mode, or rare tiny values play out differently. On ARM (the Steam Frame)
//! the same mode is FPCR's flush-to-zero bit, which treats tiny inputs and
//! results alike, as the two x86 bits together do.

/// MXCSR flush-to-zero (`0x8000`) and denormals-are-zero (`0x0040`).
pub const FTZ_DAZ: u32 = 0x8040;

/// Switches flush-to-zero and denormals-are-zero on for the current thread
/// while it lives, and restores the previous mode when dropped.
#[cfg(target_arch = "x86_64")]
#[must_use = "the mode is restored when this is dropped"]
pub struct VpxFloatMode {
    old: u32,
}

#[cfg(target_arch = "x86_64")]
impl VpxFloatMode {
    pub fn enter() -> Self {
        let old = read_mxcsr();
        write_mxcsr(old | FTZ_DAZ);
        Self { old }
    }
}

#[cfg(target_arch = "x86_64")]
impl Drop for VpxFloatMode {
    fn drop(&mut self) {
        write_mxcsr(self.old);
    }
}

#[cfg(target_arch = "x86_64")]
fn read_mxcsr() -> u32 {
    let mut v: u32 = 0;
    // SAFETY: stores the current thread's SSE control register into a local.
    unsafe { std::arch::asm!("stmxcsr [{}]", in(reg) &mut v, options(nostack, preserves_flags)) };
    v
}

#[cfg(target_arch = "x86_64")]
fn write_mxcsr(v: u32) {
    // SAFETY: loads the current thread's SSE control register; only the rounding/denormal
    // mode bits this module manages are changed from their previous value.
    unsafe { std::arch::asm!("ldmxcsr [{}]", in(reg) &v, options(nostack, preserves_flags)) };
}

/// FPCR's flush-to-zero bit (FZ): tiny inputs and results of single and double
/// precision act as zero, as MXCSR's two bits together make them on x86.
#[cfg(target_arch = "aarch64")]
const FPCR_FZ: u64 = 1 << 24;

/// Switches flush-to-zero on for the current thread while it lives, and
/// restores the previous mode when dropped.
#[cfg(target_arch = "aarch64")]
#[must_use = "the mode is restored when this is dropped"]
pub struct VpxFloatMode {
    old: u64,
}

#[cfg(target_arch = "aarch64")]
impl VpxFloatMode {
    pub fn enter() -> Self {
        let old = read_fpcr();
        write_fpcr(old | FPCR_FZ);
        Self { old }
    }
}

#[cfg(target_arch = "aarch64")]
impl Drop for VpxFloatMode {
    fn drop(&mut self) {
        write_fpcr(self.old);
    }
}

#[cfg(target_arch = "aarch64")]
fn read_fpcr() -> u64 {
    let v: u64;
    // SAFETY: reads the current thread's floating-point control register.
    unsafe { std::arch::asm!("mrs {}, fpcr", out(reg) v, options(nomem, nostack, preserves_flags)) };
    v
}

#[cfg(target_arch = "aarch64")]
fn write_fpcr(v: u64) {
    // SAFETY: writes the current thread's floating-point control register; only the
    // flush-to-zero bit this module manages differs from its previous value.
    unsafe { std::arch::asm!("msr fpcr, {}", in(reg) v, options(nomem, nostack, preserves_flags)) };
}

#[cfg(all(test, any(target_arch = "x86_64", target_arch = "aarch64")))]
mod tests {
    use super::*;

    #[test]
    fn denormals_act_as_zero_inside_and_not_after() {
        let tiny = std::hint::black_box(f32::from_bits(1)); // the smallest denormal
        {
            let _mode = VpxFloatMode::enter();
            assert_eq!(std::hint::black_box(tiny * 1.0), 0.0);
        }
        assert_eq!(std::hint::black_box(tiny * 1.0).to_bits(), 1);
    }
}
