//! GBE — the O2's display engine, the "Graphics Backend" half of CRIME.
//!
//! GBE owns everything between the framebuffer and the monitor: it reads the
//! tile-based framebuffer through a set of window descriptors, converts pixel
//! values through WID/DID and a cmap/gamma ramp, overlays the hardware cursor
//! and drives the video timing out to the DAC.
//!
//! ## Bring-up status
//!
//! [`GBE_READY`] is the single flag that says whether this is more than a
//! register stub. While it is false the PROM's `console=d` path only needs
//! GBE to answer its ID register (`GBE_CTRLSTAT`, which must read back
//! `0x000000a1` or `initGraphics` gives up) and to accept the two writes
//! `turnOffGbeDma` makes. That is what this module implements today.
//!
//! What lands next, in order (docs/o2-ip32.md §7 step 4):
//!
//! 1. video timing (`vt_xy`, `vt_vtcnt`, `vt_hcnt`) and the `vt_` registers
//!    the PROM programs per mode;
//! 2. the tile/framebuffer descriptor lists the MRE draws into;
//! 3. WID/DID, the cmap and the gamma ramp, so a pixel value means a colour;
//! 4. the cursor registers the PROM's `initCursor` programs;
//! 5. DMA out of the framebuffer, which is what makes a picture at all — and
//!    the point at which this becomes a `disp` framebuffer source and the
//!    O2's graphics console becomes usable.
//!
//! Until step 5 the emulator offers the O2's serial console only
//! (`[machine] console = "serial"`, the default), which is why the GUI greys
//! out the graphics option rather than letting a user boot into a black
//! window.

use parking_lot::Mutex;
use std::sync::atomic::{AtomicU32, Ordering};
use std::sync::Arc;

use crate::traits::BusDevice;

/// GBE's register file base, physical (`0x16000000`).
pub const GBE_BASE: u32 = 0x1600_0000;

/// Whether GBE is more than the ID register.
///
/// Read by the GUI to decide whether the O2's graphics console can be
/// offered, and by `MachineConfig::validate` — which refuses
/// `machine.console = "graphics"` while this is false rather than booting a
/// guest into a window with nothing in it.
///
/// Flip this when the display pipeline lands; nothing else needs to change,
/// because it is the one place both the GUI and the validator ask.
pub const GBE_READY: bool = false;

// ---- register offsets (GBE's own 1 MB window) ----------------------------
//
// Only the ones the PROM touches before it has a framebuffer are named here.

/// `GBE_CTRLSTAT` — control/status. The PROM reads the revision nibble out of
/// it and refuses to continue unless it is `0xa1`, so this is the first
/// register that has to be right.
pub const GBE_CTRLSTAT: u32 = 0x00;

/// The value a real GBE reports in `GBE_CTRLSTAT`'s low byte: revision `a`
/// in the high nibble of that byte. The PROM's `initGraphics` compares the
/// whole word against `0x000000a1` and jumps to the "no graphics" path on a
/// mismatch, which is the serial-console path we want.
pub const GBE_CTRLSTAT_REV: u32 = 0x0000_00a1;

/// `GBE_VT_XY` — the video-timing window size the PROM writes while turning
/// DMA off. Accepted and ignored for now: there is no scanout to size.
pub const GBE_VT_XY: u32 = 0x08;

/// `GBE_VT_VPY` / `GBE_VT_VPB` — viewport position and bounds.
pub const GBE_VT_VPY: u32 = 0x10;
pub const GBE_VT_VPB: u32 = 0x18;

/// `GBE_OVR_CONTROL` — overlay window control.
pub const GBE_OVR_CONTROL: u32 = 0x20;

/// The GBE stub: enough registers for the PROM to conclude "there is a GBE
/// here, and its revision is one I know", and to write its two timing
/// registers without faulting.
///
/// Registers are atomics rather than plain fields because `BusDevice` takes
/// `&self` and the device is shared behind an `Arc<Mutex<..>>` by the whole
/// machine — the same interior-mutability shape every other iris device uses.
///
/// Everything else in the window reads as bus noise (`u32::MAX`) rather than
/// as zero, which is what an unimplemented register on this bus looks like:
/// the PROM's `console=d` path never touches them, and if a future step
/// reaches one by accident it fails loudly instead of silently succeeding.
#[derive(Debug)]
pub struct Gbe {
    ctrlstat: AtomicU32,
    vt_xy: AtomicU32,
}

impl Gbe {
    pub fn new() -> Arc<Mutex<Self>> {
        Arc::new(Mutex::new(Self {
            ctrlstat: AtomicU32::new(GBE_CTRLSTAT_REV),
            vt_xy: AtomicU32::new(0),
        }))
    }

    /// Whether the display engine is more than the ID register. Lets callers
    /// ask the device rather than the compile-time constant, for the day the
    /// answer differs.
    pub fn is_ready(&self) -> bool {
        GBE_READY
    }
}

impl Default for Gbe {
    fn default() -> Self {
        Self {
            ctrlstat: AtomicU32::new(GBE_CTRLSTAT_REV),
            vt_xy: AtomicU32::new(0),
        }
    }
}

impl BusDevice for Gbe {
    fn read32(&self, off: u32) -> crate::traits::BusRead32 {
        let data = match off {
            GBE_CTRLSTAT => self.ctrlstat.load(Ordering::Relaxed),
            GBE_VT_XY => self.vt_xy.load(Ordering::Relaxed),
            _ => u32::MAX,
        };
        crate::traits::BusRead32::ok(data)
    }

    fn write32(&self, off: u32, val: u32) -> u32 {
        match off {
            // The status bits are read-only; only the enable bits take a
            // write, and with no scanout to enable there is nothing to record.
            GBE_CTRLSTAT => {}
            GBE_VT_XY => self.vt_xy.store(val, Ordering::Relaxed),
            _ => {}
        }
        0
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn stub_answers_the_id_the_prom_demands() {
        let gbe = Gbe::default();
        // `initGraphics` compares the whole word, not just the revision
        // nibble: a GBE that reports the right revision in the wrong place
        // takes the serial-console path by accident.
        assert_eq!(gbe.read32(GBE_CTRLSTAT).data, GBE_CTRLSTAT_REV);
        assert!(!gbe.is_ready());
    }

    #[test]
    fn unimplemented_registers_read_as_bus_noise() {
        let gbe = Gbe::default();
        assert_eq!(gbe.read32(GBE_VT_VPY).data, u32::MAX);
        assert_eq!(gbe.read32(0x100).data, u32::MAX);
    }

    #[test]
    fn the_proms_timing_write_is_accepted() {
        let gbe = Gbe::default();
        assert_eq!(gbe.write32(GBE_VT_XY, 0x04ff_03ff), 0);
        assert_eq!(gbe.read32(GBE_VT_XY).data, 0x04ff_03ff);
    }
}
