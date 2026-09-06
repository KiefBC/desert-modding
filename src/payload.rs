//! Byte layout of the PickUpItem payload (platform independent, unit tested).
//! docs/cdloot-internals.md section 3.2, "kind 3" (gather).

pub const PICKUP_DESCRIPTOR: &str = "TrocTrProcessPickUpItemOnceTimer";
/// Values the reference mod compiled in; re-resolved at start, logged if different.
pub const PICKUP_ID_EXPECTED: u16 = 2057;
pub const PICKUP_PAYLOAD_SIZE: usize = 13;
/// Payload byte 3 for gathering (the reference mod's "kind 3").
pub const GATHER_MODE: u8 = 5;
pub const GATHER_TAIL: u32 = 0xFF01_0000;

pub fn gather_payload(id: u16, target_eid: u32) -> [u8; PICKUP_PAYLOAD_SIZE] {
    let mut p = [0u8; PICKUP_PAYLOAD_SIZE];
    p[0..2].copy_from_slice(&id.to_le_bytes());
    p[2] = 0xFF;
    p[3] = GATHER_MODE;
    p[4..8].copy_from_slice(&target_eid.to_le_bytes());
    p[8..12].copy_from_slice(&GATHER_TAIL.to_le_bytes());
    p[12] = 0;
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
        let p = gather_payload(2057, 0xB012_3456);
        assert_eq!(p, [0x09, 0x08, 0xFF, 0x05, 0x56, 0x34, 0x12, 0xB0, 0x00, 0x00, 0x01, 0xFF, 0x00]);
        assert_eq!(hex(&p[..3]), "09 08 FF");
    }
}
