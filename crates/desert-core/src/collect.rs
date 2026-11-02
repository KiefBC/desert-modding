//! Gather-node records of Crimson Desert build 26361232, generated from the
//! game's own archive tables (gimmickinfo, iteminfo, interactioninfo and
//! knowledgeinfo) by the family rule in
//! crates/tools/src/bin/gen-collect-names/classify.rs, plus the overrides in
//! crates/tools/extra-families.json (records no field marks, the exclusions,
//! and the Money family). A row is a record that pays at least one output
//! block the record-loader hook multiplies, as `(record key, record name,
//! family, display name)`; the display name is the record's English name
//! from the game's own string table. Regenerate with
//! crates/tools/target/x86_64-unknown-linux-gnu/release/gen-collect-names.
//!
//! That generator rewrites this whole file and it is safe to run straight
//! over it: it owns the enum, the rows, both lookups AND the tests below,
//! including the `family_by_name` note and `non_gather_records_stay_out`.
//! Anything hand-added HERE dies on the next run - add it to the generator
//! instead.

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Family {
    /// Rule: the record's first interaction override is `Gimmick_Collect` or
    /// `Gimmick_Scoop`. Also the water drawn from a WATER WELL:
    /// `gimmick_well_0001_parts01` (key 1001081, item 22008, a fixed 5..5),
    /// whose interaction is `Gimmick_Fetch_Water` and so no rule picks it; it
    /// is an override in crates/tools/extra-families.json, which the
    /// gen-collect-names tool checks against the game's own gimmickinfo table
    /// on every run. Water from a well is gathered out of the world like
    /// everything else here, so it lives here rather than in a family of its
    /// own. The seven Wild Rock Honey records (`seockcheong_01`..`07`) are
    /// overrides too: no interaction override and no knowledge, so no field
    /// marks them. The other water source, the breakable pot
    /// `Background_Breakable_66`, and the market-stall goods hand the player a
    /// pre-built item instance and never read their record, so a table edit
    /// cannot reach them; they are listed as `records_not_enabled` in the same
    /// file and the `foraging_records_not_enabled_stay_out` test below keeps
    /// them out. See docs/findings/2026-09-12-water-wells.md section 11.
    Foraging,

    /// Rule: `Gimmick_PickUp`, no `knowledgeInfo`, and every item the record
    /// pays has the `wood` material. Not the three timber donation boxes
    /// (`gimmick_box_donation_reward_wood_01`..`03`): the rule picks them
    /// (`Gimmick_PickUp`, no knowledge, wood), but they are where the player
    /// gives timber, not where it is gathered, so they are
    /// `records_not_enabled` and the `logging_records_not_enabled_stay_out`
    /// test below keeps them out.
    Logging,

    /// Rule: `collectFilter_Dev == 2`. Also the three breakable stalactites,
    /// `cave_break_stalac_02`, `gimmick_stalactite_0003` and
    /// `gimmick_stalactite_0004` (item 1000541): overrides in
    /// crates/tools/extra-families.json, because none has an interaction
    /// override and their `collectFilter_Dev` is 0, so no field marks them.
    Mining,

    /// Rule: `Gimmick_PickUp` with a non-zero `knowledgeInfo`.
    Ore,

    /// The CURRENCY multiplier: the money props placed in the world, and
    /// nothing else. Item 1 is the game's one money item (the bag names it
    /// `Money_Copper`; copper, silver and gold are display units over one
    /// count, so 2500 copper reads as 25 silver), and these are the only
    /// gimmickinfo records paying it that are pickups. A placed coin prop reads
    /// its output block - three vanilla hand pickups of
    /// `gimmick_item_common_coin_0001` on 2026-09-13, build 25246367, paid x15,
    /// x14, x15 inside its 10..15 block, and one pickup at Money=3 the same day
    /// paid x30 against a bag delta of exactly +30
    /// (docs/findings/2026-09-12-water-wells.md section 13.7; CONFIRMED IN GAME
    /// on build 25246367) - so the same block edit that multiplies a bush
    /// multiplies them, and `hook::reapply` is what reached them live. This is
    /// the ONLY family allowed to pay item 1, and every record in it must pay
    /// item 1 and nothing else; the gen-collect-names tool enforces both
    /// halves. What it reaches is money lying in the world and nothing else:
    /// not enemy drops, not quest rewards, not coin pouches, not chests, not
    /// shop prices, not dispatch rewards. The three donation-box coin records
    /// are not pickups and are kept out below.
    Money,
}

pub const COLLECT_RECORDS: &[(u32, &str, Family, &str)] = &[
    (1003687, "amaranth_01", Family::Foraging, "Amaranth"),
    (17020034, "apple_01", Family::Foraging, "Apple"),
    (1002645, "Asparagus_01", Family::Foraging, "Asparagus"),
    (1002648, "Barley_01", Family::Foraging, "Barley"),
    (1002996, "bean_01", Family::Foraging, "Beans"),
    (17020001, "begonia_01", Family::Foraging, "Begonia"),
    (17020030, "blackberry_01", Family::Foraging, "Blackberry"),
    (17020029, "blueberry_01", Family::Foraging, "Blueberry"),
    (1005401, "Buckwheat_01", Family::Foraging, "Buckwheat"),
    (17010002, "bush_sage_01", Family::Foraging, "Croton"),
    (17010003, "bush_sage_01_red", Family::Foraging, "Red Croton"),
    (1002508, "cabbage_01", Family::Foraging, "Cabbage"),
    (1002644, "Cacao_01", Family::Foraging, "Cacao"),
    (17020021, "carrot_01", Family::Foraging, "Carrot"),
    (2530061, "cave_break_stalac_02", Family::Mining, "Stalactite"),
    (1005631, "cd_gimmick_ct_mushroom_hericium_wood_brokentree_01_parts01", Family::Foraging, "Coral Mushroom"),
    (1005632, "cd_gimmick_ct_mushroom_phellinus_wood_brokentree_01_parts01", Family::Foraging, "Pine Mushroom"),
    (1005633, "cd_gimmick_ct_mushroom_shiitake_wood_brokentree_01_parts01", Family::Foraging, "Oakwood Mushroom"),
    (1005634, "cd_gimmick_ct_poisonmushroom_wood_brokentree_01_parts01", Family::Foraging, "Grisette Mushroom"),
    (1003627, "Chaya_01", Family::Foraging, "Chaya"),
    (1002333, "Chlorella_01", Family::Foraging, "Green Algae"),
    (1002643, "Coffee_01", Family::Foraging, "Coffee Cherry"),
    (1010597, "Collect_Bamboo_Branch_01", Family::Foraging, "Rough Bamboo Stalk"),
    (1010598, "Collect_Bamboo_Branch_Hard_01", Family::Foraging, "Rough and Sturdy Bamboo Stalk"),
    (1010595, "Collect_Wood_Branch_01", Family::Foraging, "Rough Tree Branch"),
    (1010584, "Collect_Wood_Branch_Hard_01", Family::Foraging, "Rough and Sturdy Tree Branch"),
    (17020024, "corn_01", Family::Foraging, "Corn"),
    (17020009, "cotton_01", Family::Foraging, "Cotton"),
    (1002827, "cucumber_01", Family::Foraging, "Cucumber"),
    (17020032, "desertwatermelon_01", Family::Foraging, "Desert Melon"),
    (12030001, "docking_core_small", Family::Foraging, "Creature Power Core"),
    (1000844, "Dulse_01", Family::Foraging, "Red Seaweed"),
    (1002651, "Elderberry_01", Family::Foraging, "Elderberry"),
    (1003625, "Ensete_01", Family::Foraging, "Enset"),
    (1002647, "Fennel_01", Family::Foraging, "Fennel"),
    (1002642, "Figs_01", Family::Foraging, "Fig"),
    (1002971, "firewood_0001", Family::Logging, "Timber"),
    (1006738, "firewood_0001_Fine", Family::Logging, "Fine Timber"),
    (1003241, "firewood_0001_index01", Family::Logging, "Timber"),
    (1006736, "firewood_0001_index01_Fine", Family::Logging, "Fine Timber"),
    (1006737, "firewood_0001_index01_Premium", Family::Logging, "Flawless Timber"),
    (1003242, "firewood_0001_index02", Family::Logging, "Timber"),
    (1006742, "firewood_0001_index02_Fine", Family::Logging, "Fine Timber"),
    (1006743, "firewood_0001_index02_Premium", Family::Logging, "Flawless Timber"),
    (1003243, "firewood_0001_index03", Family::Logging, "Timber"),
    (1006740, "firewood_0001_index03_Fine", Family::Logging, "Fine Timber"),
    (1006741, "firewood_0001_index03_Premium", Family::Logging, "Flawless Timber"),
    (1010009, "firewood_0001_index04", Family::Logging, "Timber"),
    (1010010, "firewood_0001_index04_Fine", Family::Logging, "Fine Timber"),
    (1010011, "firewood_0001_index04_Premium", Family::Logging, "Flawless Timber"),
    (1010268, "firewood_0001_index05", Family::Logging, "Timber"),
    (1010269, "firewood_0001_index05_Fine", Family::Logging, "Fine Timber"),
    (1010270, "firewood_0001_index05_Premium", Family::Logging, "Flawless Timber"),
    (1010271, "firewood_0001_index06", Family::Logging, "Timber"),
    (1010272, "firewood_0001_index06_Fine", Family::Logging, "Fine Timber"),
    (1010273, "firewood_0001_index06_Premium", Family::Logging, "Flawless Timber"),
    (1006739, "firewood_0001_Premium", Family::Logging, "Flawless Timber"),
    (1003244, "firewood_0002", Family::Logging, "Timber"),
    (1006744, "firewood_0002_Fine", Family::Logging, "Fine Timber"),
    (1003245, "firewood_0002_index01", Family::Logging, "Timber"),
    (1006746, "firewood_0002_index01_Fine", Family::Logging, "Fine Timber"),
    (1006747, "firewood_0002_index01_Premium", Family::Logging, "Flawless Timber"),
    (1003246, "firewood_0002_index02", Family::Logging, "Timber"),
    (1006764, "firewood_0002_index02_Fine", Family::Logging, "Fine Timber"),
    (1006765, "firewood_0002_index02_Premium", Family::Logging, "Flawless Timber"),
    (1010027, "firewood_0002_index03", Family::Logging, "Timber"),
    (1010028, "firewood_0002_index03_Fine", Family::Logging, "Fine Timber"),
    (1010029, "firewood_0002_index03_Premium", Family::Logging, "Flawless Timber"),
    (1010030, "firewood_0002_index04", Family::Logging, "Timber"),
    (1010031, "firewood_0002_index04_Fine", Family::Logging, "Fine Timber"),
    (1010032, "firewood_0002_index04_Premium", Family::Logging, "Flawless Timber"),
    (1010033, "firewood_0002_index05", Family::Logging, "Timber"),
    (1010034, "firewood_0002_index05_Fine", Family::Logging, "Fine Timber"),
    (1010035, "firewood_0002_index05_Premium", Family::Logging, "Flawless Timber"),
    (1010080, "firewood_0002_index06", Family::Logging, "Timber"),
    (1010081, "firewood_0002_index06_Fine", Family::Logging, "Fine Timber"),
    (1010082, "firewood_0002_index06_Premium", Family::Logging, "Flawless Timber"),
    (1010083, "firewood_0002_index07", Family::Logging, "Timber"),
    (1010084, "firewood_0002_index07_Fine", Family::Logging, "Fine Timber"),
    (1010085, "firewood_0002_index07_Premium", Family::Logging, "Flawless Timber"),
    (1010123, "firewood_0002_index08", Family::Logging, "Timber"),
    (1010124, "firewood_0002_index08_Fine", Family::Logging, "Fine Timber"),
    (1010125, "firewood_0002_index08_Premium", Family::Logging, "Flawless Timber"),
    (1006745, "firewood_0002_Premium", Family::Logging, "Flawless Timber"),
    (1003247, "firewood_0003", Family::Logging, "Timber"),
    (1006748, "firewood_0003_Fine", Family::Logging, "Fine Timber"),
    (1003248, "firewood_0003_index01", Family::Logging, "Timber"),
    (1006750, "firewood_0003_index01_Fine", Family::Logging, "Fine Timber"),
    (1006751, "firewood_0003_index01_Premium", Family::Logging, "Flawless Timber"),
    (1003249, "firewood_0003_index02", Family::Logging, "Timber"),
    (1006766, "firewood_0003_index02_Fine", Family::Logging, "Fine Timber"),
    (1006767, "firewood_0003_index02_Premium", Family::Logging, "Flawless Timber"),
    (1010086, "firewood_0003_index03", Family::Logging, "Timber"),
    (1010087, "firewood_0003_index03_Fine", Family::Logging, "Fine Timber"),
    (1010088, "firewood_0003_index03_Premium", Family::Logging, "Flawless Timber"),
    (1006749, "firewood_0003_Premium", Family::Logging, "Flawless Timber"),
    (1000674, "firewood_1001", Family::Logging, "Timber"),
    (1000877, "firewood_1001_Fine", Family::Logging, "Fine Timber"),
    (1010036, "firewood_1001_index01", Family::Logging, "Timber"),
    (1010037, "firewood_1001_index01_Fine", Family::Logging, "Fine Timber"),
    (1010038, "firewood_1001_index01_Premium", Family::Logging, "Flawless Timber"),
    (1010126, "firewood_1001_index02", Family::Logging, "Timber"),
    (1010127, "firewood_1001_index02_Fine", Family::Logging, "Fine Timber"),
    (1010128, "firewood_1001_index02_Premium", Family::Logging, "Flawless Timber"),
    (1010286, "firewood_1001_index03", Family::Logging, "Timber"),
    (1010287, "firewood_1001_index03_Fine", Family::Logging, "Fine Timber"),
    (1010288, "firewood_1001_index03_Premium", Family::Logging, "Flawless Timber"),
    (1010289, "firewood_1001_index04", Family::Logging, "Timber"),
    (1010290, "firewood_1001_index04_Fine", Family::Logging, "Fine Timber"),
    (1010291, "firewood_1001_index04_Premium", Family::Logging, "Flawless Timber"),
    (1000973, "firewood_1001_Premium", Family::Logging, "Flawless Timber"),
    (1003206, "firewood_1002", Family::Logging, "Timber"),
    (1006752, "firewood_1002_Fine", Family::Logging, "Fine Timber"),
    (1003250, "firewood_1002_index01", Family::Logging, "Timber"),
    (1006754, "firewood_1002_index01_Fine", Family::Logging, "Fine Timber"),
    (1006755, "firewood_1002_index01_Premium", Family::Logging, "Flawless Timber"),
    (1003251, "firewood_1002_index02", Family::Logging, "Timber"),
    (1006756, "firewood_1002_index02_Fine", Family::Logging, "Fine Timber"),
    (1006757, "firewood_1002_index02_Premium", Family::Logging, "Flawless Timber"),
    (1009163, "firewood_1002_index03", Family::Logging, "Timber"),
    (1006758, "firewood_1002_index03_Fine", Family::Logging, "Fine Timber"),
    (1006759, "firewood_1002_index03_Premium", Family::Logging, "Flawless Timber"),
    (1007827, "firewood_1002_index04", Family::Logging, "Timber"),
    (1007828, "firewood_1002_index04_Fine", Family::Logging, "Fine Timber"),
    (1007829, "firewood_1002_index04_Premium", Family::Logging, "Flawless Timber"),
    (1010012, "firewood_1002_index05", Family::Logging, "Timber"),
    (1010013, "firewood_1002_index05_Fine", Family::Logging, "Fine Timber"),
    (1010014, "firewood_1002_index05_Premium", Family::Logging, "Flawless Timber"),
    (1010068, "firewood_1002_index06", Family::Logging, "Timber"),
    (1010069, "firewood_1002_index06_Fine", Family::Logging, "Fine Timber"),
    (1010070, "firewood_1002_index06_Premium", Family::Logging, "Flawless Timber"),
    (1010071, "firewood_1002_index07", Family::Logging, "Timber"),
    (1010072, "firewood_1002_index07_Fine", Family::Logging, "Fine Timber"),
    (1010073, "firewood_1002_index07_Premium", Family::Logging, "Flawless Timber"),
    (1010074, "firewood_1002_index08", Family::Logging, "Timber"),
    (1010075, "firewood_1002_index08_Fine", Family::Logging, "Fine Timber"),
    (1010076, "firewood_1002_index08_Premium", Family::Logging, "Flawless Timber"),
    (1010274, "firewood_1002_index09", Family::Logging, "Timber"),
    (1010275, "firewood_1002_index09_Fine", Family::Logging, "Fine Timber"),
    (1010276, "firewood_1002_index09_Premium", Family::Logging, "Flawless Timber"),
    (1010277, "firewood_1002_index10", Family::Logging, "Timber"),
    (1010278, "firewood_1002_index10_Fine", Family::Logging, "Fine Timber"),
    (1010279, "firewood_1002_index10_Premium", Family::Logging, "Flawless Timber"),
    (1006753, "firewood_1002_Premium", Family::Logging, "Flawless Timber"),
    (1009553, "firewood_1003", Family::Logging, "Timber"),
    (1009554, "firewood_1003_Fine", Family::Logging, "Fine Timber"),
    (1010283, "firewood_1003_index01", Family::Logging, "Timber"),
    (1010284, "firewood_1003_index01_Fine", Family::Logging, "Fine Timber"),
    (1010285, "firewood_1003_index01_Premium", Family::Logging, "Flawless Timber"),
    (1009555, "firewood_1003_Premium", Family::Logging, "Flawless Timber"),
    (1010015, "firewood_2001", Family::Logging, "Timber"),
    (1010016, "firewood_2001_Fine", Family::Logging, "Fine Timber"),
    (1010018, "firewood_2001_index01", Family::Logging, "Timber"),
    (1010019, "firewood_2001_index01_Fine", Family::Logging, "Fine Timber"),
    (1010020, "firewood_2001_index01_Premium", Family::Logging, "Flawless Timber"),
    (1010021, "firewood_2001_index02", Family::Logging, "Timber"),
    (1010022, "firewood_2001_index02_Fine", Family::Logging, "Fine Timber"),
    (1010023, "firewood_2001_index02_Premium", Family::Logging, "Flawless Timber"),
    (1010280, "firewood_2001_index03", Family::Logging, "Timber"),
    (1010281, "firewood_2001_index03_Fine", Family::Logging, "Fine Timber"),
    (1010282, "firewood_2001_index03_Premium", Family::Logging, "Flawless Timber"),
    (1010017, "firewood_2001_Premium", Family::Logging, "Flawless Timber"),
    (1003252, "firewood_2002", Family::Logging, "Timber"),
    (1006760, "firewood_2002_Fine", Family::Logging, "Fine Timber"),
    (1003253, "firewood_2002_index01", Family::Logging, "Timber"),
    (1006762, "firewood_2002_index01_Fine", Family::Logging, "Fine Timber"),
    (1006763, "firewood_2002_index01_Premium", Family::Logging, "Flawless Timber"),
    (1010024, "firewood_2002_index02", Family::Logging, "Timber"),
    (1010025, "firewood_2002_index02_Fine", Family::Logging, "Fine Timber"),
    (1010026, "firewood_2002_index02_Premium", Family::Logging, "Flawless Timber"),
    (1010077, "firewood_2002_index03", Family::Logging, "Timber"),
    (1010078, "firewood_2002_index03_Fine", Family::Logging, "Fine Timber"),
    (1010079, "firewood_2002_index03_Premium", Family::Logging, "Flawless Timber"),
    (1006761, "firewood_2002_Premium", Family::Logging, "Flawless Timber"),
    (1003254, "firewood_2003", Family::Logging, "Timber"),
    (1006768, "firewood_2003_Fine", Family::Logging, "Fine Timber"),
    (1006769, "firewood_2003_Premium", Family::Logging, "Flawless Timber"),
    (17020022, "garlic_01", Family::Foraging, "Garlic"),
    (1000183, "gimmick_item_common_coin_0001", Family::Money, "Coins"),
    (1006419, "gimmick_item_common_coin_0002", Family::Money, "Coin"),
    (1000712, "gimmick_item_common_silverbar_0001", Family::Money, "Silver Bar"),
    (1000856, "Gimmick_Item_RazorClam_0001", Family::Foraging, "Razor Clam"),
    (1001261, "gimmick_kudzu_vine_0001", Family::Foraging, "Skyroot Vine"),
    (1004924, "gimmick_stalactite_0003", Family::Mining, "Stalactite"),
    (1004925, "gimmick_stalactite_0004", Family::Mining, "Stalactite"),
    (1001081, "gimmick_well_0001_parts01", Family::Foraging, "Bucket"),
    (1010558, "gimmick_wood_firewood_0001", Family::Logging, "Firewood"),
    (17010010, "ginseng_01", Family::Foraging, "Wild Ginseng"),
    (1001538, "ginseng_02", Family::Foraging, "Ginseng"),
    (17020035, "grape_01", Family::Foraging, "Grape"),
    (17020011, "lavender_01", Family::Foraging, "Lavender"),
    (17020012, "lavender_01_blue", Family::Foraging, "Blue Lavender"),
    (17020013, "lavender_01_white", Family::Foraging, "White Lavender"),
    (17020014, "lavender_01_yellow", Family::Foraging, "Yellow Lavender"),
    (1002830, "lentils_01", Family::Foraging, "Lentils"),
    (17020028, "linum_01", Family::Foraging, "Flax"),
    (17020003, "marigold_01", Family::Foraging, "Marigold"),
    (17030001, "mine_bluestone", Family::Mining, "Azurite Mineral"),
    (17030021, "mine_bluestone_cave", Family::Mining, "Azurite Mineral"),
    (17030002, "mine_copper", Family::Mining, "Copper Mineral"),
    (17030020, "mine_copper_cave", Family::Mining, "Copper Mineral"),
    (17030003, "mine_diamond", Family::Mining, "Diamond Mineral"),
    (1000445, "mine_diamond_attach", Family::Mining, "Diamond Mineral for Docking"),
    (17030019, "mine_diamond_cave", Family::Mining, "Diamond Mineral"),
    (17030004, "mine_gold", Family::Mining, "Gold Mineral"),
    (17030018, "mine_gold_cave", Family::Mining, "Gold Mineral"),
    (17030005, "mine_greenstone", Family::Mining, "Epidote mineral"),
    (17030017, "mine_greenstone_cave", Family::Mining, "Epidote mineral"),
    (17030006, "mine_iron", Family::Mining, "Iron Mineral"),
    (1000446, "mine_iron_attach", Family::Mining, "Iron Mineral for Docking"),
    (17030016, "mine_iron_cave", Family::Mining, "Iron Mineral"),
    (17030007, "mine_redstone", Family::Mining, "Bloodstone Mineral"),
    (1000447, "mine_redstone_attach", Family::Mining, "Bloodstone Mineral for Docking"),
    (17030015, "mine_redstone_cave", Family::Mining, "Bloodstone Mineral"),
    (1002637, "mine_rock_01", Family::Mining, "Boulder Mineral"),
    (1002638, "mine_rock_02", Family::Mining, "Boulder Mineral"),
    (1002639, "mine_rock_03", Family::Mining, "Boulder Mineral"),
    (1001350, "mine_rock_04", Family::Mining, "Boulder Mineral"),
    (1001351, "mine_rock_05", Family::Mining, "Boulder Mineral"),
    (1001352, "mine_rock_06", Family::Mining, "Boulder Mineral"),
    (1001353, "mine_rock_07", Family::Mining, "Boulder Mineral"),
    (1001354, "mine_rock_08", Family::Mining, "Boulder Mineral"),
    (1001482, "mine_rock_09", Family::Mining, "Boulder Mineral"),
    (17030008, "mine_ruby", Family::Mining, "Garnet Mineral"),
    (17030014, "mine_ruby_cave", Family::Mining, "Garnet Mineral"),
    (17030009, "mine_silver", Family::Mining, "Silver Mineral"),
    (17030013, "mine_silver_cave", Family::Mining, "Silver Mineral"),
    (1001255, "mine_stone_01", Family::Mining, "Rock"),
    (17030010, "mine_whitestone", Family::Mining, "Scolecite Mineral"),
    (17030012, "mine_whitestone_cave", Family::Mining, "Scolecite Mineral"),
    (17010008, "mushroom_hericium_01", Family::Foraging, "Coral Mushroom"),
    (17010006, "mushroom_phellinus_01", Family::Foraging, "Pine Mushroom"),
    (17010007, "mushroom_shiitake_01", Family::Foraging, "Oakwood Mushroom"),
    (1002828, "navet_01", Family::Foraging, "Turnip"),
    (1002894, "Oleander_01", Family::Foraging, "Rosebay"),
    (17020004, "oliganthus_01", Family::Foraging, "Lily of the Valley"),
    (17020023, "onion_01", Family::Foraging, "Onion"),
    (1003626, "opuntia_01", Family::Foraging, "Prickly Pear"),
    (17020037, "orange_01", Family::Foraging, "Orange"),
    (170800011, "ore_bismuth_01", Family::Ore, "[EMPTY]"),
    (17080003, "ore_BlueStone_01", Family::Ore, "[EMPTY]"),
    (17080005, "ore_copper_01", Family::Ore, "[EMPTY]"),
    (17080006, "ore_diamond_01", Family::Ore, "[EMPTY]"),
    (17080007, "ore_gold_01", Family::Ore, "[EMPTY]"),
    (17080002, "ore_GreenStone_01", Family::Ore, "[EMPTY]"),
    (17070001, "ore_iron_01", Family::Ore, "[EMPTY]"),
    (17080001, "ore_RedStone_01", Family::Ore, "[EMPTY]"),
    (17080009, "ore_ruby_01", Family::Ore, "[EMPTY]"),
    (170800010, "ore_silver_01", Family::Ore, "[EMPTY]"),
    (1001256, "ore_stone_01", Family::Ore, "[EMPTY]"),
    (1001728, "ore_stone_02", Family::Ore, "[EMPTY]"),
    (1001739, "ore_stone_03", Family::Ore, "[EMPTY]"),
    (1000889, "ore_sulfurstone_01", Family::Ore, "[EMPTY]"),
    (17080004, "ore_WhiteStone_01", Family::Ore, "[EMPTY]"),
    (17020005, "pansy_01", Family::Foraging, "Pansy"),
    (1002831, "parsley_01", Family::Foraging, "Parsley"),
    (17020036, "peach_01", Family::Foraging, "Peach"),
    (1003705, "pear_02", Family::Foraging, "Pear"),
    (17020020, "peas_01", Family::Foraging, "Peas"),
    (17020006, "peony_01", Family::Foraging, "Peony"),
    (1002640, "Pineapple_01", Family::Foraging, "Pineapple"),
    (1002641, "Pomegranate_01", Family::Foraging, "Pomegranate"),
    (17020025, "potato_01", Family::Foraging, "Potato"),
    (1002511, "pumpkin_01", Family::Foraging, "Pumpkin"),
    (1004933, "Rare_Collect_Kudzu", Family::Foraging, "Skyroot"),
    (1004935, "Rare_Collect_Rubber", Family::Foraging, "Rubber"),
    (1004936, "Rare_Collect_Stalactites", Family::Ore, "[EMPTY]"),
    (17020031, "raspberry_01", Family::Foraging, "Raspberry"),
    (1002829, "redbeet_01", Family::Foraging, "Beet"),
    (17020010, "rock_orchids_01", Family::Foraging, "Dendrobium Orchid"),
    (17020015, "rosemary_01", Family::Foraging, "Rosemary"),
    (17020016, "rosemary_01_pink", Family::Foraging, "Pink Rosemary"),
    (17020017, "rosemary_01_white", Family::Foraging, "White Rosemary"),
    (17020018, "rosemary_01_yellow", Family::Foraging, "Yellow Rosemary"),
    (1006310, "rubber_Seed_0002", Family::Foraging, "Rubber Tree Seed"),
    (17060001, "seockcheong_01", Family::Foraging, "Wild Rock Honey"),
    (17060002, "seockcheong_02", Family::Foraging, "Wild Rock Honey"),
    (17060003, "seockcheong_03", Family::Foraging, "Wild Rock Honey"),
    (17060004, "seockcheong_04", Family::Foraging, "Wild Rock Honey"),
    (17060005, "seockcheong_05", Family::Foraging, "Wild Rock Honey"),
    (17060006, "seockcheong_06", Family::Foraging, "Wild Rock Honey"),
    (17060007, "seockcheong_07", Family::Foraging, "Wild Rock Honey"),
    (1002652, "Strawberry_01", Family::Foraging, "Strawberry"),
    (17020026, "sweetpotato_01", Family::Foraging, "Sweet Potato"),
    (1003688, "Taro_01", Family::Foraging, "Taro"),
    (17020027, "tomato_01", Family::Foraging, "Tomato"),
    (17020007, "variegatum_01", Family::Foraging, "Snapdragon"),
    (17020019, "veratrum_01", Family::Foraging, "False Hellebore"),
    (1002646, "Victorialis_01", Family::Foraging, "Wild Garlic"),
    (17020008, "villosa_01", Family::Foraging, "Dunbaria"),
    (17010004, "weed_croton_04", Family::Foraging, "Satintail Root"),
    (17010005, "weed_harsam_01", Family::Foraging, "False Starwort"),
    (17010001, "weed_sophora_01", Family::Foraging, "Shrubby Sophora"),
    (1002649, "Wheat_01", Family::Foraging, "Wheat"),
];

/// Family of a gather record by its key, if it is one.
pub fn family_by_key(key: u32) -> Option<Family> {
    COLLECT_RECORDS.iter().find(|(k, _, _, _)| *k == key).map(|(_, _, f, _)| *f)
}

/// Family of a gather record by its name (case-insensitive), if it is one.
/// Exact record names only. Tried and rejected (2026-09-06): treating the
/// felled-tree chunks (`log_*`) as Logging. The pickup event is ignored for
/// them; the chunk actor even survives having its `firewood_*` materials cut
/// out. Only the materials are gather nodes.
pub fn family_by_name(name: &str) -> Option<Family> {
    COLLECT_RECORDS.iter().find(|(_, n, _, _)| n.eq_ignore_ascii_case(name)).map(|(_, _, f, _)| *f)
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn non_gather_records_stay_out() {
        assert_eq!(family_by_name("log_1002_index07"), None);
        assert_eq!(family_by_name("gimmick_tree_cd_crop_apple_02_collect"), None);
        assert_eq!(family_by_name("item_basic_onehand"), None);
        assert_eq!(family_by_name("firewood_1002_index07"), Some(Family::Logging));
    }

    #[test]
    fn known_records() {
        assert_eq!(family_by_name("peony_01"), Some(Family::Foraging));
        assert_eq!(family_by_key(17020006), Some(Family::Foraging));
        assert_eq!(family_by_name("ore_copper_01"), Some(Family::Ore));
        assert_eq!(family_by_name("gimmick_gate_metal_lattice_01_dungeon"), None);
        assert_eq!(COLLECT_RECORDS.len(), 294);
    }

    /// Override rows from crates/tools/extra-families.json: records the rule in
    /// gen-collect-names/classify.rs does not pick.
    /// Items paid: 22008, 750041.
    #[test]
    fn foraging_extra_records_are_present() {
        let extra: &[(u32, &str)] = &[
            (1001081, "gimmick_well_0001_parts01"),
            (17060001, "seockcheong_01"),
            (17060002, "seockcheong_02"),
            (17060003, "seockcheong_03"),
            (17060004, "seockcheong_04"),
            (17060005, "seockcheong_05"),
            (17060006, "seockcheong_06"),
            (17060007, "seockcheong_07"),
        ];
        for &(key, name) in extra {
            assert_eq!(family_by_key(key), Some(Family::Foraging), "{name}");
            assert_eq!(family_by_name(name), Some(Family::Foraging), "{name}");
        }
        assert_eq!(
            COLLECT_RECORDS.iter().filter(|(_, _, f, _)| *f == Family::Foraging).count(),
            97
        );
    }

    /// `records_not_enabled` in crates/tools/extra-families.json: records a table
    /// edit was measured or argued not to reach - they hand the player a
    /// pre-built item instance and never read their output block (see
    /// docs/findings/2026-09-12-water-wells.md section 11), or they are
    /// not pickups at all; each record's `why` in that file says which.
    /// They must stay OUT of the table, not sit in it inert: a row here
    /// would be an edit the log reports and the player never sees.
    #[test]
    fn foraging_records_not_enabled_stay_out() {
        let out: &[(u32, &str)] = &[
            (21030076, "Background_Breakable_66"),
            (1001130, "gimmick_item_trade_cheese_01"),
            (1001167, "gimmick_item_trade_cheese_02"),
            (1001131, "gimmick_item_trade_fishmeat_01"),
            (1001168, "gimmick_item_trade_fishmeat_02"),
            (1001160, "gimmick_item_trade_flour_02"),
            (1001120, "gimmick_item_trade_flour_03"),
            (1001129, "gimmick_item_trade_ginseng_01"),
            (1001166, "gimmick_item_trade_ginseng_02"),
            (1001123, "gimmick_item_trade_honey_01"),
            (1001128, "gimmick_item_trade_pepper_01"),
            (1001132, "gimmick_item_trade_pepper_03"),
            (1001162, "gimmick_item_trade_salt_02"),
            (1001127, "gimmick_item_trade_salt_03"),
            (1001122, "gimmick_item_trade_sugar_01"),
            (1001118, "gimmick_item_trade_sugar_03"),
        ];
        for &(key, name) in out {
            assert_eq!(family_by_key(key), None, "{name}");
            assert_eq!(family_by_name(name), None, "{name}");
        }
    }

    /// `records_not_enabled` in crates/tools/extra-families.json: records a table
    /// edit was measured or argued not to reach - they hand the player a
    /// pre-built item instance and never read their output block (see
    /// docs/findings/2026-09-12-water-wells.md section 11), or they are
    /// not pickups at all; each record's `why` in that file says which.
    /// They must stay OUT of the table, not sit in it inert: a row here
    /// would be an edit the log reports and the player never sees.
    #[test]
    fn logging_records_not_enabled_stay_out() {
        let out: &[(u32, &str)] = &[
            (1005870, "gimmick_box_donation_reward_wood_01"),
            (1005871, "gimmick_box_donation_reward_wood_02"),
            (1005872, "gimmick_box_donation_reward_wood_03"),
        ];
        for &(key, name) in out {
            assert_eq!(family_by_key(key), None, "{name}");
            assert_eq!(family_by_name(name), None, "{name}");
        }
    }

    /// Override rows from crates/tools/extra-families.json: records the rule in
    /// gen-collect-names/classify.rs does not pick.
    /// Items paid: 1000541.
    #[test]
    fn mining_extra_records_are_present() {
        let extra: &[(u32, &str)] = &[
            (2530061, "cave_break_stalac_02"),
            (1004924, "gimmick_stalactite_0003"),
            (1004925, "gimmick_stalactite_0004"),
        ];
        for &(key, name) in extra {
            assert_eq!(family_by_key(key), Some(Family::Mining), "{name}");
            assert_eq!(family_by_name(name), Some(Family::Mining), "{name}");
        }
        assert_eq!(
            COLLECT_RECORDS.iter().filter(|(_, _, f, _)| *f == Family::Mining).count(),
            36
        );
    }

    /// Override rows from crates/tools/extra-families.json: records the rule in
    /// gen-collect-names/classify.rs does not pick.
    /// Items paid: 1.
    #[test]
    fn money_extra_records_are_present() {
        let extra: &[(u32, &str)] = &[
            (1000183, "gimmick_item_common_coin_0001"),
            (1006419, "gimmick_item_common_coin_0002"),
            (1000712, "gimmick_item_common_silverbar_0001"),
        ];
        for &(key, name) in extra {
            assert_eq!(family_by_key(key), Some(Family::Money), "{name}");
            assert_eq!(family_by_name(name), Some(Family::Money), "{name}");
        }
        assert_eq!(
            COLLECT_RECORDS.iter().filter(|(_, _, f, _)| *f == Family::Money).count(),
            3
        );
    }

    /// `records_not_enabled` in crates/tools/extra-families.json: records a table
    /// edit was measured or argued not to reach - they hand the player a
    /// pre-built item instance and never read their output block (see
    /// docs/findings/2026-09-12-water-wells.md section 11), or they are
    /// not pickups at all; each record's `why` in that file says which.
    /// They must stay OUT of the table, not sit in it inert: a row here
    /// would be an edit the log reports and the player never sees.
    #[test]
    fn money_records_not_enabled_stay_out() {
        let out: &[(u32, &str)] = &[
            (1005717, "gimmick_box_donation_reward_coin_01"),
            (1005718, "gimmick_box_donation_reward_coin_02"),
            (1005719, "gimmick_box_donation_reward_coin_03"),
        ];
        for &(key, name) in out {
            assert_eq!(family_by_key(key), None, "{name}");
            assert_eq!(family_by_name(name), None, "{name}");
        }
    }
}
