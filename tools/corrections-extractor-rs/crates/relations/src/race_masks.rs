//! Questie's requiredRaces / requiredClasses bitmasks. Race masks use bits above 32 (Forever's
//! Skyborne races), so masks are `u64`.

/// Questie race mask bit per Blizzard race id. Not simply 2^(id - 1): Skyborne (95, 96) use bits
/// 32 and 33. Mirrors QuestieDB `src/corrections/enum/expansions.lua` raceMaskById.
pub fn race_bit(race_id: i64) -> Option<u64> {
    Some(match race_id {
        1 => 1,        // Human
        2 => 2,        // Orc
        3 => 4,        // Dwarf
        4 => 8,        // Night Elf
        5 => 16,       // Undead
        6 => 32,       // Tauren
        7 => 64,       // Gnome
        8 => 128,      // Troll
        9 => 256,      // Goblin
        10 => 512,     // Blood Elf
        11 => 1024,    // Draenei
        22 => 1 << 21, // Worgen
        24 => 1 << 23, // Pandaren
        25 => 1 << 24, // Pandaren (Alliance)
        26 => 1 << 25, // Pandaren (Horde)
        95 => 1 << 32, // Skyborne (Alliance)
        96 => 1 << 33, // Skyborne (Horde)
        _ => return None,
    })
}

/// Class bits are 2^(classId - 1); None for ids that have no bit.
pub fn class_bit(class_id: i64) -> Option<u64> {
    (1..=64).contains(&class_id).then(|| 1u64 << (class_id - 1))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn skyborne_races_use_bits_above_32() {
        assert_eq!(race_bit(95), Some(4_294_967_296));
        assert_eq!(race_bit(96), Some(8_589_934_592));
        assert_eq!(race_bit(12), None);
    }

    #[test]
    fn class_bits_start_at_one_for_warriors() {
        assert_eq!(class_bit(1), Some(1));
        assert_eq!(class_bit(11), Some(1024));
        assert_eq!(class_bit(0), None);
    }
}
