//! Profile wiring tests for IP22/IP24/IP28 and GR2/IMPACT board selection.
//!
//! These run in `cargo test --lib` without booting the guest. For live boot
//! checks, point `iris-indigo2-smoke-ci.toml` at a local IRIX root disk (raw or
//! CHD; monitor on 127.0.0.1:8888; CI serial is channel B / IRIX).

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use parking_lot::Mutex;

    use crate::config::{
        Cli, CpuModel, GraphicsBoard, MachineConfig, MachineProfile,
    };
    use crate::dev::eeprom_93c56::Eeprom93c56;
    use crate::dev::ioc::{Ioc, IOC_BASE, IOC_SYS_ID, l1_regs, IOC_INT3_L1_STAT};
    use crate::dev::mgras::{Mgras, GIO_ID, MGRAS_SLOT_GFX_BASE};
    use crate::traits::{BusDevice, Saveable};
    use crate::dev::gr2::{Gr2, Gr2Stats, Gr2Variant, GR2_BASE};

    fn minimal_cfg() -> MachineConfig {
        let mut cfg = MachineConfig::default();
        cfg.scsi.clear();
        cfg
    }

    #[test]
    fn indigo2_smoke_ci_toml_parses_and_validates() {
        let path = concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/irix-install/iris-indigo2-smoke-ci.toml"
        );
        let text = std::fs::read_to_string(path).expect("smoke ci toml");
        let cfg: MachineConfig = toml::from_str(&text).expect("parse smoke toml");
        cfg.validate().expect("smoke config validates");
        assert_eq!(cfg.machine.profile, MachineProfile::Indigo2Ip22);
        assert!(cfg.headless);
        assert!(cfg.ci);
    }

    #[test]
    fn impact_solid_on_indigo2_validates() {
        let mut cfg = minimal_cfg();
        cfg.machine.profile = MachineProfile::Indigo2Ip22;
        cfg.graphics.board = GraphicsBoard::SolidImpact;
        cfg.validate().expect("Solid IMPACT on Indigo2");
    }

    #[test]
    fn impact_on_indy_validates() {
        let mut cfg = minimal_cfg();
        cfg.machine.profile = MachineProfile::IndyIp24;
        cfg.graphics.board = GraphicsBoard::SolidImpact;
        cfg.validate().expect("Solid IMPACT on Indy");
    }

    #[test]
    fn newport_on_ip28_validates() {
        let mut cfg = minimal_cfg();
        cfg.machine.profile = MachineProfile::Indigo2Ip28;
        cfg.graphics.board = GraphicsBoard::Newport;
        cfg.validate().expect("Newport on IP28");
    }

    #[test]
    fn xz_on_ip28_validates() {
        let mut cfg = minimal_cfg();
        cfg.machine.profile = MachineProfile::Indigo2Ip28;
        cfg.graphics.board = GraphicsBoard::Xz;
        cfg.validate().expect("XZ on IP28");
    }

    #[test]
    fn impact_keywords_parse() {
        assert_eq!("solidimpact".parse::<GraphicsBoard>().unwrap(), GraphicsBoard::SolidImpact);
        assert_eq!("impact:solid".parse::<GraphicsBoard>().unwrap(), GraphicsBoard::SolidImpact);
        assert_eq!("highimpact".parse::<GraphicsBoard>().unwrap(), GraphicsBoard::HighImpact);
        assert_eq!("impact:high".parse::<GraphicsBoard>().unwrap(), GraphicsBoard::HighImpact);
        assert_eq!("maximpact".parse::<GraphicsBoard>().unwrap(), GraphicsBoard::MaxImpact);
        assert_eq!("impact:max".parse::<GraphicsBoard>().unwrap(), GraphicsBoard::MaxImpact);
    }

    #[test]
    fn cli_graphics_keywords() {
        use clap::Parser;
        let cli = Cli::try_parse_from(["iris", "--graphics", "impact:high"]).unwrap();
        let cfg = cli.apply(MachineConfig::default());
        assert_eq!(cfg.graphics.board, GraphicsBoard::HighImpact);

        let cli = Cli::try_parse_from(["iris", "--board", "solidimpact"]).unwrap();
        let cfg = cli.apply(MachineConfig::default());
        assert_eq!(cfg.graphics.board, GraphicsBoard::SolidImpact);
    }

    #[test]
    fn xz_on_indy_validates() {
        let mut cfg = minimal_cfg();
        cfg.machine.profile = MachineProfile::IndyIp24;
        cfg.graphics.board = GraphicsBoard::Xz;
        cfg.graphics.heads = 1;
        cfg.validate().expect("XZ board on Indy");
    }

    #[test]
    fn xz_on_indigo2_validates() {
        let mut cfg = minimal_cfg();
        cfg.machine.profile = MachineProfile::Indigo2Ip22;
        cfg.graphics.board = GraphicsBoard::Xz;
        cfg.validate().expect("XZ board on Indigo2");
    }

    #[test]
    fn extreme_on_indy_rejected() {
        let mut cfg = minimal_cfg();
        cfg.machine.profile = MachineProfile::IndyIp24;
        cfg.graphics.board = GraphicsBoard::Extreme;
        let err = cfg.validate().unwrap_err();
        assert!(err.contains("indigo2_ip22"), "got: {err}");
    }

    #[test]
    fn dual_head_indigo2_validates() {
        let mut cfg = minimal_cfg();
        cfg.machine.profile = MachineProfile::Indigo2Ip22;
        cfg.graphics.heads = 2;
        cfg.validate().expect("dual-head Indigo2");
    }

    // ---- O2 (IP32) ------------------------------------------------------
    //
    // Only the profile plumbing is testable until the machine itself lands
    // (docs/o2-ip32.md §7 step 2). These cover exactly that: the profile is
    // real, it selects the IP32 PROM, and validation rejects the combinations
    // real hardware cannot have.

    /// A minimal, valid O2 config: the R5000 (the O2's stock CPU) on the GBE
    /// board it has, with the serial console.
    fn o2_cfg() -> MachineConfig {
        let mut cfg = minimal_cfg();
        cfg.machine.profile = MachineProfile::O2Ip32;
        cfg.machine.cpu = CpuModel::R5000;
        cfg.graphics.board = GraphicsBoard::Gbe;
        cfg
    }

    #[test]
    fn o2_profile_is_in_all_and_labels_itself() {
        assert!(MachineProfile::ALL.contains(&MachineProfile::O2Ip32));
        assert_eq!(MachineProfile::O2Ip32.label(), "SGI O2 (IP32)");
        // The O2 is neither an Indy nor an Indigo2, even though its name
        // suggests otherwise in passing: it is its own ASIC set.
        assert!(!MachineProfile::O2Ip32.guinness());
        assert!(!MachineProfile::O2Ip32.ip28());
        assert!(MachineProfile::O2Ip32.ip32());
        assert!(!MachineProfile::IndyIp24.ip32());
        assert!(!MachineProfile::Indigo2Ip22.ip32());
        assert!(!MachineProfile::Indigo2Ip28.ip32());
    }

    #[test]
    fn o2_cli_flag_selects_the_profile() {
        use clap::Parser;
        let cli = Cli::try_parse_from(["iris", "--o2"]).unwrap();
        let cfg = cli.apply(MachineConfig::default());
        assert_eq!(cfg.machine.profile, MachineProfile::O2Ip32);
    }

    #[test]
    fn o2_bank_sizes_are_the_dimm_sizes() {
        // CRIME's DIMMs are 32 or 128 MB; the PROM's probe sizes anything else
        // as an empty slot (docs/o2-ip32.md §1.2).
        assert_eq!(MachineProfile::O2Ip32.valid_bank_sizes(), &[0, 32, 128]);
        assert!(MachineProfile::O2Ip32.valid_bank_sizes().contains(&32));
        assert!(!MachineProfile::O2Ip32.valid_bank_sizes().contains(&64));
        assert!(!MachineProfile::O2Ip32.valid_bank_sizes().contains(&256));
        // The fullhouse profiles keep their range, and only the IP28's
        // reaches past 128 MB.
        assert_eq!(
            MachineProfile::IndyIp24.valid_bank_sizes(),
            &[0, 8, 16, 32, 64, 128]
        );
        assert!(MachineProfile::Indigo2Ip28.valid_bank_sizes().contains(&512));
    }

    #[test]
    fn o2_defaults_to_the_serial_console() {
        // The graphics console needs GBE, which does not exist yet; the
        // default has to be the UART so a bare `--o2` is not a trap
        // (docs/o2-ip32.md §2.2).
        let mut cfg = o2_cfg();
        assert_eq!(cfg.machine.console(), crate::config::ConsoleDevice::Serial);
        cfg.machine.console = Some(crate::config::ConsoleDevice::Graphics);
        assert_eq!(cfg.machine.console(), crate::config::ConsoleDevice::Graphics);
        // Other profiles are unaffected: their own firmware picks the console.
        cfg.machine.profile = MachineProfile::IndyIp24;
        assert_eq!(cfg.machine.console(), crate::config::ConsoleDevice::Graphics);
    }

    #[test]
    fn o2_128mb_dimm_configuration_validates() {
        let mut cfg = o2_cfg();
        cfg.banks = [128, 128, 0, 0];
        cfg.validate().expect("2 × 128 MB O2 DIMMs");
        cfg.banks = [32, 32, 32, 32];
        cfg.validate().expect("4 × 32 MB O2 DIMMs");
        cfg.banks = [0, 0, 0, 0];
        cfg.validate().expect("no DIMMs is a legal (if useless) O2");
    }

    #[test]
    fn o2_rejects_non_dimm_bank_sizes() {
        for bad in [8, 16, 64, 256, 512] {
            let mut cfg = o2_cfg();
            cfg.banks[0] = bad;
            let err = cfg.validate().unwrap_err();
            assert!(err.contains("not an O2 DIMM"), "bank {bad} MB: got: {err}");
        }
    }

    #[test]
    fn o2_rejects_the_r4400() {
        let mut cfg = o2_cfg();
        cfg.machine.cpu = CpuModel::R4400;
        let err = cfg.validate().unwrap_err();
        assert!(err.contains("r4400") && err.contains("R5000"), "got: {err}");
        // The R5000 and the R10000 are the two the firmware accepts.
        for cpu in [CpuModel::R5000, CpuModel::R10000] {
            cfg.machine.cpu = cpu;
            cfg.validate().expect("O2 CPU");
        }
        assert!(CpuModel::R4400.supports(MachineProfile::IndyIp24));
        assert!(!CpuModel::R4400.supports(MachineProfile::O2Ip32));
    }

    #[test]
    fn o2_rejects_gio_boards() {
        for board in [
            GraphicsBoard::Newport,
            GraphicsBoard::Xz,
            GraphicsBoard::Extreme,
            GraphicsBoard::SolidImpact,
        ] {
            let mut cfg = o2_cfg();
            cfg.graphics.board = board;
            let err = cfg.validate().unwrap_err();
            assert!(
                err.contains("GIO bus the O2 does not have"),
                "{}: got: {err}",
                board.as_str()
            );
        }
    }

    #[test]
    fn gbe_is_the_o2_only_board() {
        // The console has to be explicit: the graphics console is still a
        // stub, so the default `serial` is what validates here.
        assert!(o2_cfg().validate().is_ok(), "GBE on the O2");

        // GBE on any other profile is as wrong as Newport on an O2: the
        // display is inside the O2's CRIME ASIC, not a board in a slot.
        for profile in [
            MachineProfile::IndyIp24,
            MachineProfile::Indigo2Ip22,
            MachineProfile::Indigo2Ip28,
        ] {
            let mut cfg = minimal_cfg();
            cfg.machine.profile = profile;
            cfg.graphics.board = GraphicsBoard::Gbe;
            let err = cfg.validate().unwrap_err();
            assert!(err.contains("not valid on this machine profile"), "got: {err}");
        }
    }

    #[test]
    fn o2_graphics_console_rejected_until_gbe_lands() {
        let mut cfg = o2_cfg();
        cfg.machine.console = Some(crate::config::ConsoleDevice::Graphics);
        let err = cfg.validate().unwrap_err();
        assert!(err.contains("not emulated yet"), "got: {err}");
    }

    #[test]
    fn o2_prom_is_loaded_from_the_ip32_image() {
        // Selecting the profile is what picks the IP32 firmware; a
        // nonexistent path falls back to the embedded 4.18 image rather
        // than to an Indy's PROM.
        use crate::dev::prom::Prom;
        let prom = Prom::from_file_or_embedded_ip32("");
        let port = prom.get_port();
        // The reset vector is the first word of the image, and the image is
        // 512 KB — a full Indy PROM's worth of the wrong firmware would
        // still read as *something*, so check the IP32's own segment header.
        assert_eq!(port.read32(0x1fc00008).data & 0xffff_ffff, 0x5348_4452);
    }

    #[test]
    fn o2_flag_coerces_an_indy_config_into_a_working_o2() {
        // `iris --o2` on a bare command line: the base config is an Indy's
        // (Newport, R4400, 128 MB banks), and the flag has to produce a
        // machine that *validates*, not one that trips the profile rules.
        use clap::Parser;
        let cli = Cli::try_parse_from(["iris", "--o2"]).unwrap();
        let cfg = cli.apply(MachineConfig::default());
        assert_eq!(cfg.machine.profile, MachineProfile::O2Ip32);
        assert_eq!(cfg.graphics.board, GraphicsBoard::Gbe);
        assert_eq!(cfg.machine.cpu, CpuModel::R5000);
        assert_eq!(cfg.machine.console(), crate::config::ConsoleDevice::Serial);
        for &bank in &cfg.banks {
            assert!(cfg.machine.profile.valid_bank_sizes().contains(&bank), "bank {bank}");
        }
        cfg.validate().expect("`--o2` alone must validate");
    }

    #[test]
    fn o2_coercion_leaves_a_valid_o2_config_alone() {
        // Coercion is for incomplete configs, not a rewrite pass: a config
        // that already fits its profile comes back byte-identical.
        let mut cfg = o2_cfg();
        cfg.banks = [32, 128, 0, 0];
        cfg.machine.console = Some(crate::config::ConsoleDevice::Serial);
        let before = toml::to_string(&cfg).unwrap();
        cfg.coerce_to_profile();
        assert_eq!(toml::to_string(&cfg).unwrap(), before);
    }

    #[test]
    fn a_config_that_names_a_profile_and_contradicts_it_is_an_error() {
        // Coercion runs where the profile is chosen *for* the user. A file
        // that says "O2" and then asks for a Newport is a mistake to report,
        // not to silently fix — the fix belongs in the file.
        let mut cfg = o2_cfg();
        cfg.graphics.board = GraphicsBoard::Newport;
        let err = cfg.validate().unwrap_err();
        assert!(err.contains("use gbe"), "got: {err}");
    }

    #[test]
    fn the_o2_sample_config_parses_and_validates() {
        let path = concat!(env!("CARGO_MANIFEST_DIR"), "/iris-o2.toml");
        let text = std::fs::read_to_string(path).expect("o2 sample toml");
        let cfg: MachineConfig = toml::from_str(&text).expect("parse o2 sample");
        cfg.validate().expect("o2 sample validates");
        assert_eq!(cfg.machine.profile, MachineProfile::O2Ip32);
        assert_eq!(cfg.machine.cpu, CpuModel::R5000);
        assert_eq!(cfg.graphics.board, GraphicsBoard::Gbe);
        assert_eq!(cfg.machine.console(), crate::config::ConsoleDevice::Serial);
        assert_eq!(cfg.banks, [128, 128, 128, 128]);
    }

    #[test]
    fn ioc_sys_id_matches_profile() {
        let sys = IOC_BASE + IOC_SYS_ID;
        let indy = Ioc::new_ci(true);
        assert_eq!(indy.read32(sys).data, 0x26, "Indy guinness sys_id");

        let fullhouse = Ioc::new_ci(false);
        assert_eq!(
            fullhouse.read32(sys).data,
            0x11,
            "Indigo2 fullhouse sys_id"
        );
    }

    #[test]
    fn fullhouse_vblank_routes_sg_retrace_to_l1() {
        let ioc = Ioc::new_ci(false);
        ioc.set_interrupt(crate::dev::ioc::IocInterrupt::VerticalRetrace, true);
        let l1 = ioc.read32(IOC_BASE + IOC_INT3_L1_STAT).data as u8;
        assert_ne!(l1 & l1_regs::VERTICAL_RETRACE, 0, "SG retrace should assert L1 vblank");
        ioc.set_interrupt(crate::dev::ioc::IocInterrupt::VerticalRetrace, false);
        let l1 = ioc.read32(IOC_BASE + IOC_INT3_L1_STAT).data as u8;
        assert_eq!(l1 & l1_regs::VERTICAL_RETRACE, 0);
    }

    #[test]
    fn ioc_fullhouse_gc_select_read_write() {
        let ioc = Ioc::new_ci(false);
        let gc = IOC_BASE + crate::dev::ioc::IOC_GC_SELECT;
        assert_eq!(ioc.write32(gc, 0x0F), crate::traits::BUS_OK);
        assert_eq!(ioc.read32(gc).data, 0x0F, "fullhouse gc_select round-trip");
    }

    fn gr2(variant: Gr2Variant) -> Arc<Gr2> {
        Gr2::new(variant, Gr2Stats {
            heartbeat: Arc::new(std::sync::atomic::AtomicU64::new(0)),
            fasttick: Arc::new(std::sync::atomic::AtomicU64::new(0)),
        })
    }

    /// What PROM Gr2Probe / Gr2InitInfo read (GR2.h probe section).
    #[test]
    fn gr2_probe_answers() {
        for (variant, rev) in [(Gr2Variant::Xz, 4), (Gr2Variant::Extreme, 6)] {
            let g = gr2(variant);
            assert_eq!(g.read32(GR2_BASE + 0x6a07c).data, 0xdead_beef, "hq.mystery");
            let rd0 = g.read32(GR2_BASE + 0x6c000).data;
            assert_eq!(!rd0 & 0xf, rev, "board rev");
            let rd1 = g.read32(GR2_BASE + 0x6c004).data;
            assert_eq!(rd1 & 0x30, 0x30, "24bpp + Z");
            assert_ne!(!rd1 & 3, 0, "VB rev present");
            // The kernel reads bdvers with lbu at the word address.
            assert_eq!(g.read8(GR2_BASE + 0x6c000).data, rd0 as u8);
            // Config writes must not disturb the ID.
            g.write32(GR2_BASE + 0x6c000, 0x47);
            assert_eq!(g.read32(GR2_BASE + 0x6c000).data, rd0);
        }
    }

    /// GE count probe: installed windows are RAM, the rest do not read back.
    #[test]
    fn gr2_ge_window_probe() {
        for (variant, ges) in [(Gr2Variant::Xz, 2), (Gr2Variant::Extreme, 8)] {
            let g = gr2(variant);
            let mut found = 1;
            for i in 1..8u32 {
                let a = GR2_BASE + 0x68000 + i * 0x400;
                g.write32(a, 0x1f1f_1f1f);
                g.write32(a + 127 * 4, 0x5b5b_5b5b);
                if g.read32(a).data == 0x1f1f_1f1f && g.read32(a + 127 * 4).data == 0x5b5b_5b5b {
                    found = i + 1;
                }
            }
            assert_eq!(found, ges, "{variant:?}");
        }
    }

    #[test]
    fn gr2_save_state_round_trips_shram() {
        let g = gr2(Gr2Variant::Xz);
        g.write32(GR2_BASE + 0x7fff * 4, 1);
        let saved = g.save_state();
        let h = gr2(Gr2Variant::Xz);
        h.load_state(&saved).unwrap();
        assert_eq!(h.read32(GR2_BASE + 0x7fff * 4).data, 1);
    }

    #[test]
    fn mgras_answers_the_gio_id_probe() {
        let hb = Arc::new(std::sync::atomic::AtomicU64::new(0));
        let ioc = crate::dev::ioc::Ioc::new(false);
        let m = Mgras::new(GraphicsBoard::SolidImpact, ioc, hb.clone(), hb);
        assert_eq!(m.read32(MGRAS_SLOT_GFX_BASE).data, GIO_ID);
    }

    #[test]
    fn profile_eeprom_independent_of_mc_sysid() {
        // MC SYSID is covered in mc.rs; here we sanity-check both profiles share
        // the same EEPROM path convention (no compile-time indigo2 gate).
        let eeprom = Arc::new(Mutex::new(Eeprom93c56::new()));
        let _indy_mc = crate::dev::mc::MemoryController::new(eeprom.clone(), true, [128, 128, 0, 0]);
        let _i2_mc = crate::dev::mc::MemoryController::new(eeprom, false, [128, 128, 0, 0]);
    }
}
