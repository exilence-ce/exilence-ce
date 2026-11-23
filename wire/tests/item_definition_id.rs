use wire::v1::ItemDefinition;

fn exalted_orb() -> ItemDefinition {
    ItemDefinition {
        id: 0,
        name: String::new(),
        type_line: "Exalted Orb".into(),
        frame_type: 5,
        icon: "https://web.poecdn.com/gen/image/exalted.png".into(),
        ilvl: 0,
        tier: 0,
        links: 0,
        sockets: 0,
        quality: 0,
        level: 0,
        variant: String::new(),
        corrupted: false,
        elder: false,
        shaper: false,
        blighted: false,
        beast: false,
    }
}

// Cross-checked once against `xxhsum -H3` over a hand-written encoding:
// 1a 0b "Exalted Orb" 20 05 2a 2c "https://web.poecdn.com/gen/image/exalted.png".
const EXALTED_ORB_ID: u64 = 0x3d56_a437_1333_044c;

#[test]
fn known_definition_has_pinned_id() {
    assert_eq!(exalted_orb().content_id(), EXALTED_ORB_ID);
}

#[test]
fn id_field_does_not_change_the_id() {
    let with_id = ItemDefinition {
        id: 0xdead_beef,
        ..exalted_orb()
    };
    assert_eq!(with_id.content_id(), EXALTED_ORB_ID);
}

fn exalted_orb_with(change: impl FnOnce(&mut ItemDefinition)) -> ItemDefinition {
    let mut definition = exalted_orb();
    change(&mut definition);
    definition
}

#[test]
fn every_content_field_changes_the_id() {
    // Exhaustive: a field added to ItemDefinition fails to compile here, and
    // a binding left unused fails the warnings-as-errors build, until the
    // field has a case below.
    let ItemDefinition {
        id: _,
        name,
        type_line,
        frame_type,
        icon,
        ilvl,
        tier,
        links,
        sockets,
        quality,
        level,
        variant,
        corrupted,
        elder,
        shaper,
        blighted,
        beast,
    } = exalted_orb();

    let changed = [
        ("name", exalted_orb_with(|d| d.name = name + "x")),
        (
            "type_line",
            exalted_orb_with(|d| d.type_line = type_line + "x"),
        ),
        (
            "frame_type",
            exalted_orb_with(|d| d.frame_type = frame_type + 1),
        ),
        ("icon", exalted_orb_with(|d| d.icon = icon + "x")),
        ("ilvl", exalted_orb_with(|d| d.ilvl = ilvl + 1)),
        ("tier", exalted_orb_with(|d| d.tier = tier + 1)),
        ("links", exalted_orb_with(|d| d.links = links + 1)),
        ("sockets", exalted_orb_with(|d| d.sockets = sockets + 1)),
        ("quality", exalted_orb_with(|d| d.quality = quality + 1)),
        ("level", exalted_orb_with(|d| d.level = level + 1)),
        ("variant", exalted_orb_with(|d| d.variant = variant + "x")),
        ("corrupted", exalted_orb_with(|d| d.corrupted = !corrupted)),
        ("elder", exalted_orb_with(|d| d.elder = !elder)),
        ("shaper", exalted_orb_with(|d| d.shaper = !shaper)),
        ("blighted", exalted_orb_with(|d| d.blighted = !blighted)),
        ("beast", exalted_orb_with(|d| d.beast = !beast)),
    ];
    for (field, definition) in changed {
        assert_ne!(definition.content_id(), EXALTED_ORB_ID, "changing {field}");
    }
}
