//! The Abyss's Dimensional Cores (the Artifact Siege's "Dimensional Orbs"),
//! by the NPC code their spawn names: one per artifact, fought for during
//! the siege. The meter shares when one is seen up, its HP and its kill, as
//! for a world boss (`boss_report.rs`), so a2tools.app can show a server's
//! siege as it goes. Only what a player near a core sees: its spawn, its HP
//! while in view, its death.
//!
//! From the game's tables (NpcData `M_AR*_OrcusGem_*`, "Dimensional Core",
//! and the Abyss maps' spawners), 2026-10-18.

/// (NPC code, AbyssArtifact id or 0 when it names none), sorted by code.
pub const CORES: &[(i32, u32)] = &[
    (2600078, 1001), // Lower Reshanta: Reshanta's Root
    (2600079, 0),    // Lower Reshanta (BSZ), no fixed spawn point
    (2600080, 1002), // Lower Reshanta: Sulfur Tree Island
    (2600081, 1002), // Lower Reshanta: Sulfur Tree Island, second
    (2600082, 1003), // Lower Reshanta: Archipelago of Siel's Wings
    (2600083, 1003), // Lower Reshanta: Archipelago of Siel's Wings, second
    (2600151, 0),    // Abyss_Reshanta_B (ACRA)
    (2600152, 0),    // Abyss_Reshanta_B (ACRA), second
    (2600153, 0),    // Abyss_Reshanta_B (ACRB)
    (2600516, 2001), // second layer: Reshanta's Root
    (2600517, 2002), // second layer: Sulfur Tree Island
    (2600518, 2003), // second layer: Archipelago of Siel's Wings
];

fn find(code: i32) -> Option<usize> {
    CORES.binary_search_by_key(&code, |&(npc, _)| npc).ok()
}

/// Whether an NPC code is a Dimensional Core.
pub fn is_core(code: i32) -> bool {
    find(code).is_some()
}

/// The artifact a core stands for (AbyssArtifact id), if it names one.
pub fn artifact_of(code: i32) -> Option<u32> {
    find(code).map(|i| CORES[i].1).filter(|&a| a != 0)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_list_is_sorted_and_names_each_artifact() {
        assert!(CORES.windows(2).all(|w| w[0].0 < w[1].0));
        assert_eq!(artifact_of(2600080), Some(1002));
        assert_eq!(artifact_of(2600518), Some(2003));
        assert_eq!(artifact_of(2600079), None);
        assert!(is_core(2600079) && !is_core(2600068));
    }
}
