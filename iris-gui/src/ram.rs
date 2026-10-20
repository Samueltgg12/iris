//! RAM bank helpers shared by the Memory tab, menus, and status readouts.

/// Quick preset totals (MB) for the Memory menu and new-machine dialog.
const RAM_PRESETS: &[u32] = &[32, 64, 96, 128, 192, 256, 384, 512];

const IP28_RAM_PRESETS: &[u32] = &[32, 64, 96, 128, 192, 256, 384, 512, 768, 1024];

/// The O2's CRIME can only express 32/128 MB DIMMs, so its presets are the
/// sums those actually reach.
const O2_RAM_PRESETS: &[u32] = &[32, 64, 96, 128, 160, 192, 256, 384, 512];

/// Preset totals that make sense on `profile`, given the bank sizes its
/// memory controller can express. A preset that needs a bank the profile
/// cannot have is not offered at all — offering it would only produce a
/// config `validate()` rejects at Start.
pub fn ram_presets(profile: iris::config::MachineProfile) -> &'static [u32] {
    if profile.ip32() {
        O2_RAM_PRESETS
    } else if profile.ip28() {
        IP28_RAM_PRESETS
    } else {
        RAM_PRESETS
    }
}

pub fn active_banks(banks: &[u32; 4]) -> usize {
    banks.iter().filter(|&&s| s > 0).count()
}

pub fn ram_summary(banks: &[u32; 4]) -> String {
    let total: u32 = banks.iter().sum();
    let n = active_banks(banks);
    t!("ram-summary", total = total, banks = n)
}

#[cfg(test)]
mod tests {
    use super::*;
    use iris::config::MachineProfile;

    #[test]
    fn o2_presets_are_reachable_with_its_dimm_sizes() {
        let sizes = MachineProfile::O2Ip32.valid_bank_sizes();
        for &p in ram_presets(MachineProfile::O2Ip32) {
            // Every O2 preset must be a sum of the DIMM sizes it offers, so
            // greedy-filling four banks lands exactly on it.
            assert_eq!(
                crate::dialogs::new_machine::distribute_ram(p, sizes).iter().sum::<u32>(),
                p,
                "O2 preset {p} MB is not 32/128-DIMM reachable"
            );
        }
        // 64 MB is reachable (2 × 32) even though it is not itself a DIMM,
        // while 768 needs a 256 MB bank the O2 cannot have.
        assert!(ram_presets(MachineProfile::O2Ip32).contains(&64));
        assert!(!ram_presets(MachineProfile::O2Ip32).contains(&768));
    }

    #[test]
    fn ip28_keeps_its_large_presets() {
        assert!(ram_presets(MachineProfile::Indigo2Ip28).contains(&1024));
        assert!(!ram_presets(MachineProfile::IndyIp24).contains(&1024));
    }
}
