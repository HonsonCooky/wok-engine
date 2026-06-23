//! The this-scene Instances view: the active scene tab's placements, laid out per the panel header's
//! sort toggle - grouped under their prefab as a collapsible tree (the default), or a flat A-Z list.
//!
//! A row click emits `Action::Select` and the selected row carries the full-bleed accent highlight;
//! the floating inspector (`crate::inspector`) shows the selection. There are no per-instance state
//! dots, hidden styling, or visibility toggles: per-instance physical state is not authored here
//! (editor-design.md placement boundary), the game owns it by id at runtime. The group open state is
//! egui-managed transient view memory, keyed per scene and prefab ([`instance_group_id`]) - collapsing
//! a group is a browsing affordance, never an authored edit. The pure grouping ([`group_by_prefab`]) is
//! unit-tested directly.

use wok_scene::{InstanceId, Placement};

use crate::action::Action;
use crate::icons;
use crate::loaded::LoadedScene;
use crate::model::InstanceSort;
use crate::theme;

use super::{INSTANCE_INDENT, NAV_ROW_HEIGHT, ROW_PAD, SELECTION_ALPHA, TREE_GAP, empty_note, glyph_cell};

/// The Instances view: the active scene tab's placements, laid out per the header's sort toggle -
/// grouped under their prefab as a collapsible tree ([`instances_tree`], the default), or a flat A-Z
/// list ([`instances_flat`]). A row click emits [`Action::Select`]; the row matching `selection` carries
/// the full-bleed highlight. Three resting states with nothing to list: no scene tab active
/// (`loaded_scene` is `None`) reads "No scene open"; a scene that failed to load reads "Scene failed to
/// load" (the detail is noted on the residency for a later bite to surface); and a loaded scene with no
/// placements reads "No instances".
pub(super) fn instances_list(
    ui: &mut egui::Ui,
    sort: InstanceSort,
    selection: Option<InstanceId>,
    loaded_scene: Option<&LoadedScene>,
    actions: &mut Vec<Action>,
) {
    ui.add_space(4.0);
    let Some(loaded) = loaded_scene else {
        empty_note(ui, "No scene open");
        return;
    };
    // A failed load is empty too, but it is not "no instances" - say so rather than misreport it.
    if loaded.error().is_some() {
        empty_note(ui, "Scene failed to load");
        return;
    }
    let placements = loaded.placements();
    if placements.is_empty() {
        empty_note(ui, "No instances");
        return;
    }
    // A scroll area takes over when the list outgrows the body, the same as the project-scoped lists,
    // so a placement-heavy scene reaches every row rather than clipping past the fold. The rows sit
    // flush against each other (no inter-row spacing) so the tree reads tight and the full-bleed
    // selection highlight is one continuous band, the Zed file-tree look.
    egui::ScrollArea::vertical().auto_shrink([false, false]).show(ui, |ui| {
        ui.spacing_mut().item_spacing.y = 0.0;
        match sort {
            InstanceSort::Group => instances_tree(ui, loaded.name(), placements, selection, actions),
            InstanceSort::Flat => instances_flat(ui, placements, selection, actions),
        }
    });
}

/// The Instances view's flat layout: every placement as one selectable row at the base indent, sorted
/// A-Z by its display label. The order is on the label the row reads, so a named placement sorts by its
/// name and an unnamed one by its `{prefab} #{id}` fallback - not by the underlying instance id. A click
/// emits [`Action::Select`]; the row whose id is `selection` carries the highlight.
fn instances_flat(
    ui: &mut egui::Ui,
    placements: &[Placement],
    selection: Option<InstanceId>,
    actions: &mut Vec<Action>,
) {
    let mut sorted: Vec<&Placement> = placements.iter().collect();
    sorted.sort_by_key(|p| placement_label(p));
    for placement in sorted {
        if instance_row(ui, placement, ROW_PAD, selection == Some(placement.instance_id)) {
            actions.push(Action::Select(placement.instance_id));
        }
    }
}

/// The Instances view's grouped layout: one collapsible group row per prefab (sorted by prefab name)
/// carrying its instance count, and under an open group the indented, selectable instance rows (in
/// instance-id order, the residency's order). The group open state is egui-managed transient view
/// memory, not model state - collapsing a group is a browsing affordance, never an authored edit. A
/// click on an instance row emits [`Action::Select`]; the row whose id is `selection` carries the
/// highlight.
fn instances_tree(
    ui: &mut egui::Ui,
    scene: &str,
    placements: &[Placement],
    selection: Option<InstanceId>,
    actions: &mut Vec<Action>,
) {
    for (prefab, members) in group_by_prefab(placements) {
        if group_row(ui, scene, prefab, members.len()) {
            for placement in members {
                if instance_row(ui, placement, INSTANCE_INDENT, selection == Some(placement.instance_id)) {
                    actions.push(Action::Select(placement.instance_id));
                }
            }
        }
    }
}

/// Bucket placements by their prefab, returning one entry per prefab - its name and its members - with
/// the entries sorted by prefab name. Members keep the input's order, which the residency sorts by
/// instance id, so each bucket comes out id-ordered without a second sort. Pure (no egui), so the
/// grouping is unit tested directly. A linear scan per placement is ample at editor scene scale.
fn group_by_prefab(placements: &[Placement]) -> Vec<(&str, Vec<&Placement>)> {
    let mut groups: Vec<(&str, Vec<&Placement>)> = Vec::new();
    for placement in placements {
        let prefab = placement.prefab.as_str();
        match groups.iter_mut().find(|(name, _)| *name == prefab) {
            Some((_, members)) => members.push(placement),
            None => groups.push((prefab, vec![placement])),
        }
    }
    groups.sort_by(|a, b| a.0.cmp(b.0));
    groups
}

/// One prefab group row: a folder glyph (open or closed, the disclosure - the chevron was dropped), the
/// prefab name, and the instance count on the right, in a fixed-height cell. The whole row senses a
/// click that toggles the group's open state - egui transient memory keyed per scene and prefab
/// ([`instance_group_id`]), so it outlives a frame but is never authored. Returns whether the group is
/// open, so the caller renders its instance rows. Hovering lights the row.
fn group_row(ui: &mut egui::Ui, scene: &str, prefab: &str, count: usize) -> bool {
    let p = theme::palette(ui.ctx());
    let id = instance_group_id(scene, prefab);
    let mut open = ui.data_mut(|d| d.get_temp::<bool>(id).unwrap_or(true));
    let (rect, response) =
        ui.allocate_exact_size(egui::vec2(ui.available_width(), NAV_ROW_HEIGHT), egui::Sense::click());
    let response = response.on_hover_cursor(egui::CursorIcon::PointingHand);
    if response.clicked() {
        open = !open;
        ui.data_mut(|d| d.insert_temp(id, open));
    }
    let hovered = response.hovered();
    if hovered {
        ui.painter().rect_filled(rect, 0.0, p.hover);
    }
    let ink = if hovered { p.text_bright } else { p.text };
    // [pad][folder][prefab name .......][count][pad] - the open/closed folder is the disclosure.
    let folder = if open { icons::FOLDER_OPEN } else { icons::FOLDER };
    let folder_rect = glyph_cell(rect, ROW_PAD);
    icons::paint(ui.painter(), folder_rect, folder, ink);
    let font = egui::TextStyle::Body.resolve(ui.style());
    let name_pos = egui::pos2(folder_rect.right() + TREE_GAP, rect.center().y);
    ui.painter().text(name_pos, egui::Align2::LEFT_CENTER, prefab, font.clone(), ink);
    let count_pos = egui::pos2(rect.right() - ROW_PAD, rect.center().y);
    ui.painter().text(count_pos, egui::Align2::RIGHT_CENTER, count.to_string(), font, p.text_dim);
    open
}

/// One selectable instance row: a cube glyph at `indent` and the placement's label, filling the row.
/// `selected` paints the full-bleed selection highlight (the accent at [`SELECTION_ALPHA`] across the
/// whole panel width); otherwise the row lights on hover. Returns whether it was clicked this frame, for
/// the caller to emit [`Action::Select`]. Shared by the grouped tree (indented under its group, at
/// [`INSTANCE_INDENT`]) and the flat list (at the base [`ROW_PAD`]).
fn instance_row(ui: &mut egui::Ui, placement: &Placement, indent: f32, selected: bool) -> bool {
    let p = theme::palette(ui.ctx());
    let (rect, response) =
        ui.allocate_exact_size(egui::vec2(ui.available_width(), NAV_ROW_HEIGHT), egui::Sense::click());
    let response = response.on_hover_cursor(egui::CursorIcon::PointingHand);
    let hovered = response.hovered();
    if selected {
        let fill = egui::Color32::from_rgba_unmultiplied(p.accent.r(), p.accent.g(), p.accent.b(), SELECTION_ALPHA);
        ui.painter().rect_filled(rect, 0.0, fill);
    } else if hovered {
        ui.painter().rect_filled(rect, 0.0, p.hover);
    }
    let ink = if selected || hovered { p.text_bright } else { p.text };
    let cube_rect = glyph_cell(rect, indent);
    icons::paint(ui.painter(), cube_rect, icons::CUBE, ink);
    let font = egui::TextStyle::Body.resolve(ui.style());
    let pos = egui::pos2(cube_rect.right() + TREE_GAP, rect.center().y);
    ui.painter().text(pos, egui::Align2::LEFT_CENTER, placement_label(placement), font, ink);
    response.clicked()
}

/// The egui-memory id a prefab group's open state is stored under, keyed per scene and prefab so
/// collapsing a group in one scene leaves a same-named prefab's group in another scene at its default.
/// `pub(crate)` so the snapshot test can drive a group collapsed by seeding the very id the view reads
/// (the open state is transient view memory, not model state a test could build directly).
pub(crate) fn instance_group_id(scene: &str, prefab: &str) -> egui::Id {
    egui::Id::new(("wok_instances_group", scene, prefab))
}

/// One placement's display label: its author-given name when set, else the `{prefab} #{id}` fallback
/// (e.g. `oak_tree #3`) - enough to tell two instances of the same prefab apart by their stable
/// instance id. Display formatting, so it lives view-side with the rows that show it, not on the data;
/// both the flat list and the grouped tree's instance rows read from it.
fn placement_label(placement: &Placement) -> String {
    match &placement.name {
        Some(name) => name.clone(),
        None => format!("{} #{}", placement.prefab.as_str(), placement.instance_id.0),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use wok_scene::{InstanceId, PrefabRef, Transform};

    fn placement(prefab: &str, id: u32) -> Placement {
        Placement {
            prefab: PrefabRef::new(prefab),
            instance_id: InstanceId(id),
            name: None,
            transform: Transform::IDENTITY,
            state: None,
        }
    }

    #[test]
    fn group_by_prefab_buckets_sorted_by_name_with_id_ordered_members() {
        // Input in instance-id order (the residency's order) with the prefabs interleaved and out of
        // name order, so a pass proves the bucketing and the name sort rather than incidental input
        // order: well #0, oak_tree #1, well #2, oak_tree #3, oak_tree #4.
        let placements = vec![
            placement("well", 0),
            placement("oak_tree", 1),
            placement("well", 2),
            placement("oak_tree", 3),
            placement("oak_tree", 4),
        ];
        let groups = group_by_prefab(&placements);

        // One entry per distinct prefab, sorted by prefab name (oak_tree before well), counts correct.
        let summary: Vec<(&str, usize)> = groups.iter().map(|(name, members)| (*name, members.len())).collect();
        assert_eq!(summary, vec![("oak_tree", 3), ("well", 2)]);

        // Members keep instance-id order within each group (the id-ordered input, bucketed in place).
        let oak_ids: Vec<u32> = groups[0].1.iter().map(|p| p.instance_id.0).collect();
        assert_eq!(oak_ids, vec![1, 3, 4]);
        let well_ids: Vec<u32> = groups[1].1.iter().map(|p| p.instance_id.0).collect();
        assert_eq!(well_ids, vec![0, 2]);
    }
}
