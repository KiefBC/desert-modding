//! Byte layout of the PickUpItem payload (platform independent, unit tested).
//! docs/cdloot-internals.md section 3.2, "kind 3" (gather).

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

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PickupMode {
    /// A gather node (plant, vein, rock, wood): mode byte 5.
    Gather,
    /// An item lying on the ground: mode byte 0, different tail word.
    Item,
}

pub fn pickup_payload(id: u16, target_eid: u32, mode: PickupMode) -> [u8; PICKUP_PAYLOAD_SIZE] {
    let (b3, tail) = match mode {
        PickupMode::Gather => (GATHER_MODE, GATHER_TAIL),
        PickupMode::Item => (ITEM_MODE, ITEM_TAIL),
    };
    let mut p = [0u8; PICKUP_PAYLOAD_SIZE];
    p[0..2].copy_from_slice(&id.to_le_bytes());
    p[2] = 0xFF;
    p[3] = b3;
    p[4..8].copy_from_slice(&target_eid.to_le_bytes());
    p[8..12].copy_from_slice(&tail.to_le_bytes());
    p[12] = 0;
    p
}

pub fn gather_payload(id: u16, target_eid: u32) -> [u8; PICKUP_PAYLOAD_SIZE] {
    pickup_payload(id, target_eid, PickupMode::Gather)
}

pub fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|b| format!("{b:02X}")).collect::<Vec<_>>().join(" ")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn gather_payload_layout() {
        let p = gather_payload(2057, 0xB012_3456);
        assert_eq!(p, [0x09, 0x08, 0xFF, 0x05, 0x56, 0x34, 0x12, 0xB0, 0x00, 0x00, 0x01, 0xFF, 0x00]);
        assert_eq!(hex(&p[..3]), "09 08 FF");
        let i = pickup_payload(2057, 0xB010_0642, PickupMode::Item);
        assert_eq!(i, [0x09, 0x08, 0xFF, 0x00, 0x42, 0x06, 0x10, 0xB0, 0x01, 0x01, 0x00, 0xFF, 0x00]);
    }
}
