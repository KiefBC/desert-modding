//! The F7 event recorder's cap policy: two numbers and the pure decision they
//! drive (platform independent, unit tested).
//!
//! The recorder itself lives in `crate::events`, which is `#[cfg(windows)]`
//! because it reads game memory. The policy does not, so it sits here where it
//! compiles and is tested natively, and where [`crate::config`] - also always
//! compiled - can build the menu's help text out of the same constants instead
//! of repeating them in prose that rots.

/// Global cap on **logged lines** (it used to bound observed events).
pub const RECORD_CAP: u32 = 300;

/// Per-descriptor cap on logged lines.
///
/// Sized off a real session's census (build 25246367: 1200 recorded events,
/// which the log shows as 1208 `[record]` lines once the four `ON` and four
/// cap-stop control lines are counted). `TrocTrHandleGameEventOnceTimer` alone
/// was 1046 of them (87%), while every descriptor worth recording was tiny - 60
/// `PushKnowledge`, 47 `SaveGameData`, 33 `ProcessPickUpItem`, and 4
/// `ProcessLootingDeadDrop`, the one actually being hunted. 25 is comfortably
/// above every rare descriptor's real count and cuts the flood by ~40x, so a
/// rare event can no longer be crowded out of the global cap by a common one.
/// That is the whole point of the per-descriptor cap: without it the 300-line
/// cap filled in about three seconds and one of four hand-skinned animals' loot
/// events was lost.
pub const RECORD_PER_DESC: u32 = 25;

/// What to do with one observed event.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct RecordAction {
    /// Log a line for it.
    pub log: bool,
    /// This event is the one that fills the global cap: print the cap message
    /// and the census, and switch recording off. Only ever set together with
    /// `log`, which the `stop_implies_log` test pins.
    pub stop: bool,
}

/// The whole cap policy, as a pure function of the counters: `desc_logged` is
/// how many lines this descriptor has already had, `total_logged` how many
/// lines have been logged in all, both *before* this event. The observed counts
/// do not enter into it - they are tallied for the census only, which is
/// exactly why the census can report true volume while the log stays short.
pub const fn record_action(desc_logged: u32, total_logged: u32) -> RecordAction {
    // Nothing more to log for this event, by either cap: the global one (an
    // earlier event printed the cap message and switched recording off, so this
    // one must not stop again) or this descriptor's own share, which leaves
    // recording running for everyone else.
    if total_logged >= RECORD_CAP || desc_logged >= RECORD_PER_DESC {
        return RecordAction { log: false, stop: false };
    }
    RecordAction { log: true, stop: total_logged + 1 >= RECORD_CAP }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Replay a stream of descriptor keys through [`record_action`], keeping the
    /// counters exactly as `events::on_enqueue` does. Returns the per-key logged
    /// counts, the total logged, and the stream position that stopped
    /// recording, if any.
    fn replay(keys: usize, stream: &[usize]) -> (Vec<u32>, u32, Option<usize>) {
        let mut logged = vec![0u32; keys];
        let mut total = 0;
        let mut stopped = None;
        for (i, &k) in stream.iter().enumerate() {
            if stopped.is_some() {
                break;
            }
            let per = logged.get_mut(k).expect("key in range");
            let action = record_action(*per, total);
            if action.log {
                *per += 1;
                total += 1;
            }
            if action.stop {
                stopped = Some(i);
            }
        }
        (logged, total, stopped)
    }

    #[test]
    fn first_event_of_a_descriptor_is_always_logged() {
        assert_eq!(record_action(0, 0), RecordAction { log: true, stop: false });
        // Even with the global cap nearly full, a descriptor's first line gets
        // in - which is the property the whole change exists for.
        assert!(record_action(0, RECORD_CAP - 1).log);
    }

    #[test]
    fn per_descriptor_cap_holds_with_the_global_cap_wide_open() {
        assert!(record_action(RECORD_PER_DESC - 1, 0).log);
        assert!(!record_action(RECORD_PER_DESC, 0).log);
        assert!(!record_action(RECORD_PER_DESC + 1, 0).log);
    }

    #[test]
    fn the_line_that_fills_the_global_cap_stops_exactly_once() {
        let a = record_action(0, RECORD_CAP - 1);
        assert_eq!(a, RecordAction { log: true, stop: true });
        // And nothing after it logs or stops a second time.
        for extra in 0..4 {
            assert_eq!(record_action(0, RECORD_CAP + extra), RecordAction { log: false, stop: false });
        }
    }

    #[test]
    fn stop_implies_log() {
        for desc_logged in [0, 1, RECORD_PER_DESC - 1, RECORD_PER_DESC, RECORD_PER_DESC + 1] {
            for total in [0, 1, RECORD_CAP - 2, RECORD_CAP - 1, RECORD_CAP, RECORD_CAP + 1] {
                let a = record_action(desc_logged, total);
                assert!(!a.stop || a.log, "stop without log at ({desc_logged}, {total})");
            }
        }
    }

    /// The census of the session that lost an event, build 25246367: eight
    /// descriptors over 1200 recorded events. The flood is first in the stream
    /// and the rare dead-drop events last, the worst case for them.
    const CENSUS: [(&str, u32); 8] = [
        ("TrocTrHandleGameEventOnceTimer", 1046),
        ("TrocTrPushKnowledgeOnceTimer", 60),
        ("TrocTrSaveGameDataAutoTimer", 47),
        ("TrocTrProcessPickUpItemOnceTimer", 33),
        ("TrocTrDeleteSequencerOnceTimer", 4),
        ("TrocTrUpdateStageOnceTimer", 3),
        ("TrocTrInteractionRewardOnceTimer", 3),
        ("TrocTrProcessLootingDeadDropOnceTimer", 4),
    ];

    fn census_stream() -> Vec<usize> {
        let mut s = Vec::new();
        for (i, (_, n)) in CENSUS.iter().enumerate() {
            for _ in 0..*n {
                s.push(i);
            }
        }
        s
    }

    #[test]
    fn the_rare_dead_drop_event_survives_the_flood() {
        let dead_drop = CENSUS.len() - 1;
        let stream = census_stream();
        assert_eq!(stream.len(), 1200);
        // It arrives only after 1046 flood events, far past the global cap.
        assert!(stream.len() - CENSUS[dead_drop].1 as usize > RECORD_CAP as usize);

        let (logged, total, stopped) = replay(CENSUS.len(), &stream);
        assert_eq!(logged.get(dead_drop), Some(&4), "all four dead-drop events must be logged");
        // The flood is capped, every rare descriptor is logged in full, and the
        // whole session now fits inside the global cap with room to spare.
        assert_eq!(
            logged,
            vec![RECORD_PER_DESC, RECORD_PER_DESC, RECORD_PER_DESC, RECORD_PER_DESC, 4, 3, 3, 4]
        );
        assert_eq!(total, 114);
        assert_eq!(stopped, None);
    }

    #[test]
    fn the_old_global_cap_alone_lost_it() {
        // The bug, modelled: log everything until 300 lines are out. The
        // dead-drop events never get a line, which is what was observed in game
        // and what `RECORD_PER_DESC` fixes.
        let stream = census_stream();
        let mut logged = vec![0u32; CENSUS.len()];
        for &k in &stream {
            let total: u32 = logged.iter().sum();
            if total >= RECORD_CAP {
                break;
            }
            *logged.get_mut(k).expect("key in range") += 1;
        }
        assert_eq!(logged.get(CENSUS.len() - 1), Some(&0));
    }
}
