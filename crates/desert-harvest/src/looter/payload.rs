//! Byte layout of the PickUpItem payload (platform independent, unit tested).
//! This is the "kind 3" (gather) payload.

pub const PICKUP_DESCRIPTOR: &str = "TrocTrProcessPickUpItemOnceTimer";
/// Values the reference mod compiled in; re-resolved at start, logged if different.
pub const PICKUP_ID_EXPECTED: u16 = 2057;
pub const PICKUP_PAYLOAD_SIZE: usize = 13;
/// Payload byte 3 for gathering (the reference mod's "kind 3").
pub const GATHER_MODE: u8 = 5;
pub const GATHER_TAIL: u32 = 0xFF01_0000;
/// Ground items (the mod's "kind 1"): every call site passes mode byte 0.
pub const ITEM_MODE: u8 = 0;
pub const ITEM_TAIL: u32 = 0xFF00_0101;

/// Catching an insect is a different event with a different, shorter payload
/// (the reference mod's "kind 2"). Recorded live on build 25116796 while the
/// player caught three insects by hand (`docs/reference-internals.md`
/// section 15): `u16 id LE | 0xFF | u32 target eid LE | 0x03`.
pub const CATCH_DESCRIPTOR: &str = "TrocTrPushCharacterToInventoryOnceTimer";
pub const CATCH_ID_EXPECTED: u16 = 2048;
pub const CATCH_PAYLOAD_SIZE: usize = 8;
/// The trailing byte the game itself wrote in all three recorded catches.
pub const CATCH_TAIL: u8 = 3;

/// Skinning an animal carcass is a third event again, and the shortest of the
/// three: `u16 id LE | 0xFF | u32 target eid LE`, which is [`catch_payload`]
/// without its trailing byte. The game's own queue site writes exactly those
/// seven bytes (`FUN_142a00290`, `local_28 = 0x7e8` then the eid out of
/// `carcassActor+0x60`, verified by decompile on build 25246367), and two
/// carcasses skinned by hand were recorded coming off the event queue in two
/// separate sessions (`docs/findings/2026-09-15-skinning.md` sections 1 and 3):
///
/// ```text
/// E8 07 FF AF 21 30 B0     session 1, eid 0xB03021AF
/// E8 07 FF 87 03 10 B0     session 2, eid 0xB0100387
/// ```
///
/// The same layout was read off the reference mod on build 25116796 as its
/// "kind 0" (`docs/reference-internals.md` lines 98-106), so it is confirmed on
/// two builds. The handler (`FUN_142b62d90`) reads the one u32 at payload+3 and
/// touches nothing else in the seven bytes; it grants the carcass loot in the
/// same tick, with no animation, no distance check and no quest or busy gate,
/// and re-firing is idempotent - the granted rows are erased from the
/// dead-drop component, so a second event finds an empty list and grants
/// nothing. There is no dupe here (section 2).
pub const SKIN_DESCRIPTOR: &str = "TrocTrProcessLootingDeadDropOnceTimer";
pub const SKIN_ID_EXPECTED: u16 = 2024;
pub const SKIN_PAYLOAD_SIZE: usize = 7;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PickupMode {
    /// A gather node (plant, vein, rock, wood): mode byte 5.
    Gather,
    /// An item lying on the ground: mode byte 0, different tail word.
    Item,
    /// An insect: a different descriptor and the 8-byte payload
    /// [`catch_payload`] builds, not [`pickup_payload`].
    Catch,
    /// An animal carcass: a third descriptor again and the 7-byte payload
    /// [`skin_payload`] builds, not [`pickup_payload`].
    Skin,
}

impl PickupMode {
    /// Which of the three descriptors this mode's event is built from.
    pub fn descriptor_name(self) -> &'static str {
        match self {
            PickupMode::Gather | PickupMode::Item => PICKUP_DESCRIPTOR,
            PickupMode::Catch => CATCH_DESCRIPTOR,
            PickupMode::Skin => SKIN_DESCRIPTOR,
        }
    }
}

/// The 13-byte `PickUpItem` payload. Neither `Catch` nor `Skin` is a
/// `PickUpItem` at all - each is its own event with its own shorter payload,
/// built by [`catch_payload`] and [`skin_payload`] - so asking for either here
/// returns `None` rather than a plausible-looking wrong event aimed at a live
/// actor.
pub fn pickup_payload(id: u16, target_eid: u32, mode: PickupMode) -> Option<[u8; PICKUP_PAYLOAD_SIZE]> {
    let (b3, tail) = match mode {
        PickupMode::Gather => (GATHER_MODE, GATHER_TAIL),
        PickupMode::Item => (ITEM_MODE, ITEM_TAIL),
        PickupMode::Catch | PickupMode::Skin => return None,
    };
    let mut p = [0u8; PICKUP_PAYLOAD_SIZE];
    p[0..2].copy_from_slice(&id.to_le_bytes());
    p[2] = 0xFF;
    p[3] = b3;
    p[4..8].copy_from_slice(&target_eid.to_le_bytes());
    p[8..12].copy_from_slice(&tail.to_le_bytes());
    p[12] = 0;
    Some(p)
}

/// The 8-byte `PushCharacterToInventory` payload, byte for byte as the game
/// wrote it in the three recorded catches (section 15).
pub fn catch_payload(id: u16, target_eid: u32) -> [u8; CATCH_PAYLOAD_SIZE] {
    let mut p = [0u8; CATCH_PAYLOAD_SIZE];
    p[0..2].copy_from_slice(&id.to_le_bytes());
    p[2] = 0xFF;
    p[3..7].copy_from_slice(&target_eid.to_le_bytes());
    p[7] = CATCH_TAIL;
    p
}

/// The 7-byte `ProcessLootingDeadDrop` payload, byte for byte as the game's
/// own queue site writes it: a catch payload with the tail byte left off.
pub fn skin_payload(id: u16, target_eid: u32) -> [u8; SKIN_PAYLOAD_SIZE] {
    let mut p = [0u8; SKIN_PAYLOAD_SIZE];
    p[0..2].copy_from_slice(&id.to_le_bytes());
    p[2] = 0xFF;
    p[3..7].copy_from_slice(&target_eid.to_le_bytes());
    p
}

pub fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|b| format!("{b:02X}")).collect::<Vec<_>>().join(" ")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn gather_payload_layout() {
        let p = pickup_payload(2057, 0xB012_3456, PickupMode::Gather).expect("gather has a payload");
        assert_eq!(p, [0x09, 0x08, 0xFF, 0x05, 0x56, 0x34, 0x12, 0xB0, 0x00, 0x00, 0x01, 0xFF, 0x00]);
        assert_eq!(hex(&p[..3]), "09 08 FF");
        let i = pickup_payload(2057, 0xB010_0642, PickupMode::Item).expect("item has a payload");
        assert_eq!(i, [0x09, 0x08, 0xFF, 0x00, 0x42, 0x06, 0x10, 0xB0, 0x01, 0x01, 0x00, 0xFF, 0x00]);
    }

    /// The three catches recorded in game on build 25116796 (2026-09-08),
    /// `docs/reference-internals.md` section 15. These are the game's own
    /// bytes, not a guess.
    #[test]
    fn catch_payload_matches_what_the_game_queued() {
        assert_eq!(
            catch_payload(CATCH_ID_EXPECTED, 0xB010_02C3),
            [0x00, 0x08, 0xFF, 0xC3, 0x02, 0x10, 0xB0, 0x03]
        );
        assert_eq!(
            catch_payload(CATCH_ID_EXPECTED, 0xB010_02DC),
            [0x00, 0x08, 0xFF, 0xDC, 0x02, 0x10, 0xB0, 0x03]
        );
        assert_eq!(
            catch_payload(CATCH_ID_EXPECTED, 0xB010_02DB),
            [0x00, 0x08, 0xFF, 0xDB, 0x02, 0x10, 0xB0, 0x03]
        );
    }

    /// The two carcasses recorded coming off the game's own event queue, one
    /// per session (`docs/findings/2026-09-15-skinning.md` section 1). These
    /// are the game's bytes, not a guess.
    #[test]
    fn skin_payload_matches_what_the_game_queued() {
        assert_eq!(
            skin_payload(SKIN_ID_EXPECTED, 0xB030_21AF),
            [0xE8, 0x07, 0xFF, 0xAF, 0x21, 0x30, 0xB0]
        );
        assert_eq!(
            skin_payload(SKIN_ID_EXPECTED, 0xB010_0387),
            [0xE8, 0x07, 0xFF, 0x87, 0x03, 0x10, 0xB0]
        );
        // The whole difference from a catch is the tail byte, which this event
        // does not carry: seven bytes against eight, identical up to there.
        assert_eq!(SKIN_PAYLOAD_SIZE + 1, CATCH_PAYLOAD_SIZE);
        assert_eq!(
            skin_payload(CATCH_ID_EXPECTED, 0xB010_02C3)[..],
            catch_payload(CATCH_ID_EXPECTED, 0xB010_02C3)[..SKIN_PAYLOAD_SIZE]
        );
    }

    /// A catch is not a `PickUpItem`; the 13-byte builder must refuse it.
    #[test]
    fn pickup_payload_refuses_catch() {
        assert_eq!(pickup_payload(2048, 0xB010_02C3, PickupMode::Catch), None);
        assert_eq!(PickupMode::Catch.descriptor_name(), CATCH_DESCRIPTOR);
        assert_eq!(PickupMode::Gather.descriptor_name(), PICKUP_DESCRIPTOR);
    }

    /// Nor is skinning one: the same rule and the same refusal. A `Skin` asked
    /// for the 13-byte payload must produce nothing at all, because a 13-byte
    /// dead-drop event carrying a mode byte and a tail word the handler never
    /// reads would be a wrong event pointed at a live actor.
    #[test]
    fn pickup_payload_refuses_skin() {
        assert_eq!(pickup_payload(SKIN_ID_EXPECTED, 0xB030_21AF, PickupMode::Skin), None);
        assert_eq!(PickupMode::Skin.descriptor_name(), SKIN_DESCRIPTOR);
    }
}
