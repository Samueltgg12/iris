//! O2 (IP32) graphics: CRIME's display and rasterizer halves.
//!
//! CRIME is the whole graphics ASIC, and this module owns the parts IRIS
//! emulates of it, in one place, the way `ng1/` (Newport), `gr2/` and
//! `mgras/` each own a GIO board:
//!
//! - `gbe` — the **GBE** (Graphics Backend) display engine: video timing,
//!   tile-based framebuffer addressing, WID/DID, cmap and gamma, cursor.
//! - `crime_re` — CRIME's register file at `0x15000000`, the interface the
//!   **MRE** (Media Raster Engine) and the MTE are driven through.
//!
//! The memory and interrupt halves of CRIME are *not* here: those are
//! `crate::dev::crime`, the machine's memory controller and only interrupt
//! controller, which every O2 device reports through.
//!
//! See `docs/o2-ip32.md` §3 for the bring-up order.

pub mod gbe;
